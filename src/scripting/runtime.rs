//! Starlark evaluator wiring: dialect, globals, sandbox limits, the
//! register-time validation pipeline and `compute()` execution (ADR-0024).

use serde_json::{json, Value as Json};
use starlark::environment::{FrozenModule, Globals, GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::ast::{AssignOp, AstNoPayload, ExprP, Stmt};
use starlark::syntax::{AstModule, Dialect};
use starlark::values::dict::DictRef;
use starlark::values::list::{AllocList, ListRef};
use starlark::values::range::Range;
use starlark::values::tuple::{AllocTuple, TupleRef, UnpackTuple};
use starlark::values::{Heap, Value};
use starlark_syntax::syntax::uniplate::Visit;
use std::cell::Cell;
use std::collections::HashMap;
use std::num::NonZeroI32;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::chart_builder::{Chart, Ohlcv};
use super::series::err;
use super::ScriptError;
use crate::data::OhlcvData;
use crate::indicator::PanelResult;

/// Largest accepted script source, in bytes.
pub const MAX_SOURCE: usize = 64 * 1024;
/// Largest string, list or tuple an operator (`+`, `*`, `%`), `range`, `str`
/// or `zip` may produce. Bounds what one step can allocate.
pub const MAX_SEQ: usize = 20_000;
/// Starlark ticks (function calls + loop iterations) per evaluation.
const MAX_TICKS: u64 = 20_000_000;
/// Starlark heap cap (checked every 1000 ticks; best effort upstream).
const MAX_HEAP: usize = 64 << 20;
/// Nested calls; recursion bombs stop here.
const MAX_CALLSTACK: usize = 64;
/// f64 values all series of one evaluation may allocate (256 MiB). Series
/// data lives outside the Starlark heap, so it is counted exactly here.
const MAX_SERIES_VALUES: usize = 32 << 20;
/// Wall-clock cap for `compute()` on a chart request.
pub const COMPUTE_TIMEOUT: Duration = Duration::from_millis(1000);
/// Wall-clock cap for the register-time dry run and module top level.
pub const DRY_RUN_TIMEOUT: Duration = Duration::from_millis(200);
/// Bars of synthetic OHLCV the register-time dry run uses.
pub const DRY_RUN_BARS: usize = 100;
/// Attribute names rejected statically: string/list methods whose output can
/// outgrow their inputs many times over in one step.
const BANNED_ATTRS: &[&str] = &["extend", "format", "join", "replace"];
/// Globals rejected statically (dynamic attribute access would bypass
/// `BANNED_ATTRS`).
const BANNED_IDENTS: &[&str] = &["getattr"];

// ---- Budget: deadline + series allocation, checked by every native call ----

#[derive(Clone, Copy)]
struct Budget {
    deadline: Instant,
    series_values_left: usize,
}

thread_local! {
    static BUDGET: Cell<Option<Budget>> = const { Cell::new(None) };
}

/// Charge `values` f64s of series data to the running evaluation and check
/// its deadline. Every `ta.*` function and series operator calls this, so a
/// slow native call cannot run past the deadline by more than itself.
pub(crate) fn charge(values: usize) -> starlark::Result<()> {
    let Some(mut b) = BUDGET.with(Cell::get) else {
        return Ok(());
    };
    if Instant::now() > b.deadline {
        return err("time limit exceeded");
    }
    if values > b.series_values_left {
        return err(format!(
            "memory limit exceeded: series data over {} MiB",
            (MAX_SERIES_VALUES * 8) >> 20
        ));
    }
    b.series_values_left -= values;
    BUDGET.with(|c| c.set(Some(b)));
    Ok(())
}

/// Run `f` with an evaluator limited to `timeout`, the tick, heap and call
/// stack caps, and a fresh series budget. Panics become errors.
fn limited<'v, 'a, R>(
    module: &'a Module<'v>,
    timeout: Duration,
    f: impl FnOnce(&mut Evaluator<'v, 'a, '_>) -> starlark::Result<R>,
) -> Result<R, String> {
    let deadline = Instant::now() + timeout;
    BUDGET.with(|c| {
        c.set(Some(Budget {
            deadline,
            series_values_left: MAX_SERIES_VALUES,
        }))
    });
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut eval = Evaluator::new(module);
        eval.set_max_tick_count(MAX_TICKS)
            .and_then(|_| eval.set_max_heap_size(MAX_HEAP))
            .and_then(|_| eval.set_max_callstack_size(MAX_CALLSTACK))
            .map_err(|e| e.to_string())?;
        eval.set_check_cancelled(Box::new(move || Instant::now() > deadline));
        f(&mut eval).map_err(|e| describe(&e, deadline))
    }));
    BUDGET.with(|c| c.set(None));
    match out {
        Ok(r) => r,
        Err(_) => Err("internal error while evaluating the script".into()),
    }
}

/// `message (line N)` → split later by `error_from`; keeps the timeout cause.
fn describe(e: &starlark::Error, deadline: Instant) -> String {
    let line = error_line(e);
    let mut msg = e.without_diagnostic().to_string();
    if Instant::now() > deadline && msg.contains("ancel") {
        msg = "time limit exceeded".into();
    }
    match line {
        Some(l) => format!("{l}\u{0}{msg}"),
        None => msg,
    }
}

/// First line (1-based) the error points at: its own span, else the
/// innermost call-stack frame with a location in the script.
fn error_line(e: &starlark::Error) -> Option<usize> {
    e.span()
        .or_else(|| {
            e.call_stack()
                .frames
                .iter()
                .rev()
                .find_map(|f| f.location.as_ref())
        })
        .map(|s| s.resolve_span().begin.line + 1)
}

fn error_from(step: &'static str, raw: String) -> ScriptError {
    match raw.split_once('\u{0}') {
        Some((line, msg)) => ScriptError::new(step, line.parse().ok(), msg),
        None => ScriptError::new(step, None, raw),
    }
}

// ---- Globals ----

/// Rough upper bound of `repr(v)`'s length, failing once it passes `limit`.
pub(crate) fn repr_len(v: Value, limit: usize) -> starlark::Result<usize> {
    fn go(v: Value, left: &mut usize, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        let mut take = |n: usize| {
            if n > *left {
                false
            } else {
                *left -= n;
                true
            }
        };
        if let Some(s) = v.unpack_str() {
            return take(s.len() + 2);
        }
        let children: Vec<Value> = if let Some(l) = ListRef::from_value(v) {
            l.content().to_vec()
        } else if let Some(t) = TupleRef::from_value(v) {
            t.content().to_vec()
        } else if let Some(d) = DictRef::from_value(v) {
            d.iter().flat_map(|(k, x)| [k, x]).collect()
        } else {
            return take(32);
        };
        take(2 + 2 * children.len()) && children.iter().all(|c| go(*c, left, depth + 1))
    }
    let mut left = limit;
    if go(v, &mut left, 0) {
        Ok(limit - left)
    } else {
        err(format!(
            "value too large to convert to a string (limit {limit} characters)"
        ))
    }
}

#[starlark_module]
fn guards(builder: &mut GlobalsBuilder) {
    /// `str(x)`, refusing results over `MAX_SEQ` characters.
    fn str<'v>(
        #[starlark(require = pos)] x: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        if x.unpack_str().is_some() {
            return Ok(x);
        }
        repr_len(x, MAX_SEQ)?;
        Ok(heap.alloc(x.to_str()))
    }

    /// `repr(x)`, refusing results over `MAX_SEQ` characters.
    fn repr<'v>(
        #[starlark(require = pos)] x: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        repr_len(x, MAX_SEQ)?;
        Ok(heap.alloc(x.to_repr()))
    }

    /// `range(stop)` / `range(start, stop[, step])`, at most `MAX_SEQ` long.
    fn range(
        #[starlark(require = pos)] a: i32,
        #[starlark(require = pos)] b: Option<i32>,
        #[starlark(require = pos, default = 1)] step: i32,
    ) -> starlark::Result<Range> {
        let (start, stop) = match b {
            Some(b) => (a, b),
            None => (0, a),
        };
        let Some(step) = NonZeroI32::new(step) else {
            return err("range() step must not be zero");
        };
        let len = ((stop as i64 - start as i64) / step.get() as i64).max(0);
        if len > MAX_SEQ as i64 {
            return err(format!("range() too long: {len} (limit {MAX_SEQ})"));
        }
        Ok(Range::new(start, stop, step))
    }

    /// `zip(a, b, ...)` as a list of tuples, reading at most `4 * MAX_SEQ`
    /// input values.
    fn zip<'v>(
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let limit = 4 * MAX_SEQ;
        let mut cols = Vec::with_capacity(args.items.len());
        let mut total = 0;
        for a in args.items {
            let mut col = Vec::new();
            for x in a.iterate(heap)? {
                total += 1;
                if total > limit {
                    return err(format!("zip() result too large (limit {limit} values)"));
                }
                col.push(x);
            }
            cols.push(col);
        }
        let n = cols.iter().map(Vec::len).min().unwrap_or(0);
        let rows: Vec<Value> = (0..n)
            .map(|i| heap.alloc(AllocTuple(cols.iter().map(|c| c[i]))))
            .collect();
        Ok(heap.alloc(AllocList(rows)))
    }
}

fn globals() -> &'static Globals {
    static G: OnceLock<Globals> = OnceLock::new();
    G.get_or_init(|| {
        GlobalsBuilder::standard()
            .with(guards)
            .with(super::series::operators)
            .with_namespace("ta", super::ta::ta_module)
            .build()
    })
}

fn dialect() -> Dialect {
    Dialect {
        enable_load: false,
        ..Dialect::Standard
    }
}

// ---- Validation pipeline: parse → static → (load) → presence ----

/// Script metadata from `PARAMS` and `META`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptInfo {
    pub title: String,
    pub description: String,
    pub overlay: bool,
    /// `Indicator::params` shape: `[{name, type, default, description?, min?, max?, enum?}]`.
    pub params: Vec<Json>,
}

impl ScriptInfo {
    pub fn to_json(&self) -> Json {
        json!({
            "title": self.title,
            "description": self.description,
            "overlay": self.overlay,
            "params": self.params,
        })
    }
}

/// A script after parse, static check and top-level evaluation, frozen so
/// every `compute()` call can share it.
pub struct Compiled {
    module: FrozenModule,
    pub info: ScriptInfo,
}

fn parse(name: &str, source: &str) -> Result<AstModule, ScriptError> {
    AstModule::parse(&format!("{name}.star"), source.to_owned(), &dialect()).map_err(|e| {
        let line = e.span().map(|s| s.resolve_span().begin.line + 1);
        ScriptError::new("parse", line, e.without_diagnostic().to_string())
    })
}

/// Static check over the AST; returns the byte-offset edits that turn each
/// `x op= y` into `x = x op (y)`, so augmented assignments also go through
/// the guarded `_op_*` operators.
fn static_check(ast: &AstModule, source: &str) -> Result<Vec<(usize, usize, String)>, ScriptError> {
    let reserved: std::collections::HashSet<String> =
        globals().names().map(|n| n.as_str().to_owned()).collect();
    let fail = |span: starlark::codemap::Span, msg: String| {
        let line = ast.file_span(span).resolve_span().begin.line + 1;
        Err(ScriptError::new("static", Some(line), msg))
    };

    // Top-level definitions must not shadow builtins (`ta`, `len`, ...).
    let mut top = Vec::new();
    match &ast.statement().node {
        Stmt::Statements(xs) => top.extend(xs.iter()),
        _ => top.push(ast.statement()),
    }
    for s in top {
        let names: Vec<&str> = match &s.node {
            Stmt::Def(d) => vec![d.name.ident.as_str()],
            Stmt::Assign(a) => {
                let mut v = Vec::new();
                a.lhs.visit_lvalue(|i| v.push(i.ident.as_str()));
                v
            }
            _ => vec![],
        };
        if let Some(n) = names.into_iter().find(|n| reserved.contains(*n)) {
            return fail(
                s.span,
                format!("`{n}` shadows a builtin; pick another name"),
            );
        }
    }

    let mut problem: Option<(starlark::codemap::Span, String)> = None;
    let mut edits = Vec::new();
    fn walk<'a>(
        v: Visit<'a, AstNoPayload>,
        problem: &mut Option<(starlark::codemap::Span, String)>,
        edits: &mut Vec<(usize, usize, String)>,
        source: &str,
    ) {
        if problem.is_some() {
            return;
        }
        match &v {
            Visit::Stmt(s) => match &s.node {
                Stmt::Load(_) => *problem = Some((s.span, "load() is not allowed".into())),
                Stmt::AssignModify(lhs, op, rhs) => {
                    let sym = match op {
                        AssignOp::Add => "+",
                        AssignOp::Subtract => "-",
                        AssignOp::Multiply => "*",
                        AssignOp::Divide => "/",
                        AssignOp::Percent => "%",
                        AssignOp::LeftShift => "<<",
                        _ => "",
                    };
                    if !sym.is_empty() {
                        let (l0, l1) = (
                            lhs.span.begin().get() as usize,
                            lhs.span.end().get() as usize,
                        );
                        let (r0, r1) = (
                            rhs.span.begin().get() as usize,
                            rhs.span.end().get() as usize,
                        );
                        let lhs_src = &source[l0..l1];
                        edits.push((l1, r0, format!(" = {lhs_src} {sym} (")));
                        edits.push((r1, r1, ")".into()));
                    }
                }
                _ => {}
            },
            Visit::Expr(e) => match &e.node {
                ExprP::Dot(_, attr) if BANNED_ATTRS.contains(&attr.node.as_str()) => {
                    *problem = Some((
                        e.span,
                        format!("`.{}()` is not available in scripts", attr.node),
                    ))
                }
                ExprP::Identifier(id) if BANNED_IDENTS.contains(&id.ident.as_str()) => {
                    *problem = Some((
                        e.span,
                        format!("`{}` is not available in scripts", id.ident),
                    ))
                }
                _ => {}
            },
        }
        v.visit_children(|c| walk(c, problem, edits, source));
    }
    walk(
        Visit::Stmt(ast.statement()),
        &mut problem,
        &mut edits,
        source,
    );
    if let Some((span, msg)) = problem {
        return fail(span, msg);
    }
    Ok(edits)
}

/// Apply `(start, end, replacement)` byte edits (non-overlapping).
fn apply_edits(source: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by_key(|e| std::cmp::Reverse((e.0, e.1)));
    let mut out = source.to_owned();
    for (a, b, text) in edits {
        out.replace_range(a..b, &text);
    }
    out
}

/// Parse, static-check and evaluate the module top level, then check that
/// `PARAMS`, `META` and `compute` are present and well-formed.
pub fn compile(name: &str, source: &str) -> Result<Compiled, ScriptError> {
    if source.len() > MAX_SOURCE {
        return Err(ScriptError::new(
            "parse",
            None,
            format!("source is {} bytes (limit {MAX_SOURCE})", source.len()),
        ));
    }
    let ast = parse(name, source)?;
    let edits = static_check(&ast, source)?;
    let mut ast = if edits.is_empty() {
        ast
    } else {
        parse(name, &apply_edits(source, edits))?
    };
    let ops: HashMap<String, String> = super::series::OPERATOR_FUNCTIONS
        .iter()
        .map(|(op, f)| (op.to_string(), f.to_string()))
        .collect();
    ast.replace_binary_operators(&ops);

    let module = Module::with_temp_heap(|module| {
        limited(&module, DRY_RUN_TIMEOUT, |eval| {
            eval.eval_module(ast, globals()).map(|_| ())
        })
        .map_err(|e| error_from("static", e))?;
        module
            .freeze()
            .map_err(|e| ScriptError::new("static", None, format!("{e:?}")))
    })?;
    let info = presence(name, &module)?;
    Ok(Compiled { module, info })
}

fn presence(name: &str, module: &FrozenModule) -> Result<ScriptInfo, ScriptError> {
    let fail = |msg: String| Err(ScriptError::new("presence", None, msg));
    let get = |n: &str| module.get_option(n).ok().flatten();

    match get("compute") {
        Some(f) if f.value().get_type() == "function" => {}
        Some(_) => {
            return fail("`compute` must be a function: def compute(ohlcv, params, chart)".into())
        }
        None => return fail("missing `def compute(ohlcv, params, chart)`".into()),
    }
    let to_json = |n: &str| -> Result<Option<Json>, ScriptError> {
        match get(n) {
            None => Ok(None),
            Some(v) => v
                .value()
                .to_json_value()
                .map(Some)
                .map_err(|e| ScriptError::new("presence", None, format!("{n}: {e}"))),
        }
    };
    let Some(Json::Array(params)) = to_json("PARAMS")? else {
        return fail("missing `PARAMS = [...]` list (use `PARAMS = []` for none)".into());
    };
    let Some(Json::Object(meta)) = to_json("META")? else {
        return fail("missing `META = {...}` dict".into());
    };
    let text = |k: &str| meta.get(k).and_then(|v| v.as_str()).map(str::to_owned);
    let overlay = match (meta.get("overlay"), text("category").as_deref()) {
        (Some(Json::Bool(b)), _) => *b,
        (Some(_), _) => return fail("META.overlay must be a bool".into()),
        (None, Some("overlay")) => true,
        (None, None | Some("panel")) => false,
        (None, Some(c)) => {
            return fail(format!(
                "META.category must be \"overlay\" or \"panel\", got {c:?}"
            ))
        }
    };
    if params.len() > 32 {
        return fail("at most 32 PARAMS".into());
    }
    let params = params
        .iter()
        .map(normalize_param)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|m| ScriptError::new("presence", None, format!("PARAMS: {m}")))?;
    Ok(ScriptInfo {
        title: text("title").unwrap_or_else(|| name.to_owned()),
        description: text("description").unwrap_or_default(),
        overlay,
        params,
    })
}

/// Script param spec → the `Indicator::params` schema `validate_params` reads.
fn normalize_param(p: &Json) -> Result<Json, String> {
    let name = p
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !n.is_empty())
        .ok_or("every entry needs a \"name\" (letters, digits, _)")?;
    if name == "name" {
        return Err("a param cannot be called \"name\"".into());
    }
    let ty = match p.get("type").and_then(|v| v.as_str()) {
        Some("int" | "integer") => "integer",
        Some("float" | "number") => "number",
        Some("bool" | "boolean") => "boolean",
        Some("str" | "string") => "string",
        other => {
            return Err(format!(
                "{name}: type must be int, float, bool or string, got {other:?}"
            ))
        }
    };
    let default = p.get("default").cloned().unwrap_or(Json::Null);
    let ok = match ty {
        "integer" => default.is_i64(),
        "number" => default.is_number(),
        "boolean" => default.is_boolean(),
        _ => default.is_string(),
    };
    if !ok {
        return Err(format!("{name}: default must be a {ty}"));
    }
    let mut out = json!({ "name": name, "type": ty, "default": default });
    if let Some(d) = p
        .get("desc")
        .or_else(|| p.get("description"))
        .and_then(|v| v.as_str())
    {
        out["description"] = json!(d);
    }
    for k in ["min", "max"] {
        match p.get(k) {
            None | Some(Json::Null) => {}
            Some(v) if v.is_number() => out[k] = v.clone(),
            Some(_) => return Err(format!("{name}: {k} must be a number")),
        }
    }
    if let Some(e) = p.get("options").or_else(|| p.get("enum")) {
        match e.as_array() {
            Some(a) if ty == "string" && a.iter().all(|x| x.is_string()) => out["enum"] = e.clone(),
            _ => {
                return Err(format!(
                    "{name}: options must be a list of strings on a string param"
                ))
            }
        }
    }
    Ok(out)
}

// ---- Execution ----

/// Resolved `params` dict for `compute`: configured values over defaults,
/// numbers coerced to the declared type.
fn param_values(info: &ScriptInfo, configured: &Json) -> Json {
    let mut out = serde_json::Map::new();
    for spec in &info.params {
        let name = spec["name"].as_str().unwrap_or_default();
        let v = configured
            .get(name)
            .filter(|v| !v.is_null())
            .unwrap_or(&spec["default"]);
        let v = match spec["type"].as_str() {
            Some("number") => json!(v.as_f64().unwrap_or(f64::NAN)),
            _ => v.clone(),
        };
        out.insert(name.to_owned(), v);
    }
    Json::Object(out)
}

/// Run `compute(ohlcv, params, chart)` and return the chart it drew.
pub fn run(
    compiled: &Compiled,
    data: &OhlcvData,
    configured: &Json,
    timeout: Duration,
    step: &'static str,
) -> Result<PanelResult, ScriptError> {
    let params = param_values(&compiled.info, configured);
    Module::with_temp_heap(|module| {
        let heap = module.heap();
        let compute = compiled
            .module
            .get("compute")
            .map_err(|e| ScriptError::new(step, None, e.to_string()))?;
        let compute = heap.access_owned_frozen_value(&compute);
        let chart = heap.alloc(Chart::new(data.len(), &compiled.info));
        let args = [heap.alloc(Ohlcv::new(data)), heap.alloc(params), chart];
        limited(&module, timeout, |eval| {
            let ret = eval.eval_function(compute, &args, &[])?;
            if !ret.is_none() {
                return err("compute() must not return a value; draw with chart.line(...) etc.");
            }
            Ok(())
        })
        .map_err(|e| error_from(step, e))?;
        Ok(Chart::from_value(chart)
            .map(Chart::take)
            .unwrap_or_default())
    })
}

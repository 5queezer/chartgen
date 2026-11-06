//! Starlark evaluator wiring: dialect, globals, sandbox limits, the
//! register-time validation pipeline and `compute()` execution (ADR-0024).

use serde_json::{json, Value as Json};
use starlark::codemap::Span;
use starlark::environment::{FrozenModule, Globals, GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::ast::{
    AssignTargetP, AstAssignTarget, AstExpr, AstLiteral, AstNoPayload, BinOp, ClauseP, ExprP, Stmt,
};
use starlark::syntax::{AstModule, Dialect};
use starlark::values::dict::DictRef;
use starlark::values::float::UnpackFloat;
use starlark::values::list::{AllocList, ListRef};
use starlark::values::none::NoneType;
use starlark::values::range::Range;
use starlark::values::tuple::{AllocTuple, TupleRef, UnpackTuple};
use starlark::values::{Heap, UnpackValue, Value};
use starlark_syntax::lexer::TokenInt;
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
/// Largest string, list, tuple or dict an operator (`+`, `*`, `%`, `|`),
/// `range`, `str`, `zip`, `.append`, `.insert` or `d[k] = v` may produce.
/// Bounds what one step can allocate.
pub const MAX_SEQ: usize = 20_000;
/// Starlark ticks (function calls + loop iterations) per evaluation.
const MAX_TICKS: u64 = 20_000_000;
/// Starlark heap cap (checked every 1000 ticks; best effort upstream).
const MAX_HEAP: usize = 64 << 20;
/// Nested calls; recursion bombs stop here.
const MAX_CALLSTACK: usize = 64;
/// Net memory one evaluation may hold: every allocation of the evaluating
/// thread (Starlark heap chunks, dict tables, series data, strings), counted
/// by `alloc::Counting` and checked on every native call and every 1000
/// ticks.
const MAX_MEMORY: isize = 256 << 20;
/// Wall-clock cap for `compute()` on a chart request.
pub const COMPUTE_TIMEOUT: Duration = Duration::from_millis(1000);
/// Wall-clock cap for the register-time dry run and module top level.
pub const DRY_RUN_TIMEOUT: Duration = Duration::from_millis(200);
/// Bars of synthetic OHLCV the register-time dry run uses.
pub const DRY_RUN_BARS: usize = 100;
/// Largest frozen module (top-level values) a script may leave behind; it
/// stays in the compile cache.
pub const MAX_MODULE_BYTES: usize = 1 << 20;
/// Attribute names rejected statically: string/list/dict methods whose
/// output can outgrow their inputs many times over in one step, or that
/// grow a dict past the `_setitem` cap.
const BANNED_ATTRS: &[&str] = &[
    "extend",
    "format",
    "join",
    "replace",
    "update",
    "setdefault",
];
/// Globals rejected statically (dynamic attribute access would bypass
/// `BANNED_ATTRS`).
const BANNED_IDENTS: &[&str] = &["getattr"];

// ---- Budget: deadline, memory and bar count, checked by every native call ----

#[derive(Clone, Copy)]
struct Budget {
    deadline: Instant,
    mem_base: isize,
    /// Bar count of the chart; every series must have exactly this length
    /// (0 outside `compute`, where no series may be made).
    bars: usize,
}

#[derive(Clone, Copy)]
enum Stop {
    Time,
    Memory,
}

impl Stop {
    fn message(self) -> String {
        match self {
            Stop::Time => "time limit exceeded".into(),
            Stop::Memory => format!("memory limit exceeded ({} MiB)", MAX_MEMORY >> 20),
        }
    }
}

thread_local! {
    static BUDGET: Cell<Option<Budget>> = const { Cell::new(None) };
    static STOPPED: Cell<Option<Stop>> = const { Cell::new(None) };
}

fn over(b: &Budget) -> Option<Stop> {
    if Instant::now() > b.deadline {
        Some(Stop::Time)
    } else if super::alloc::thread_net() - b.mem_base > MAX_MEMORY {
        Some(Stop::Memory)
    } else {
        None
    }
}

/// Check the running evaluation's deadline and memory. Every `ta.*`
/// function, series operation and guarded operator calls this before its
/// work, so a native call overshoots by at most its own cost.
pub(crate) fn check() -> starlark::Result<()> {
    match BUDGET.with(Cell::get).as_ref().and_then(over) {
        Some(stop) => err(stop.message()),
        None => Ok(()),
    }
}

/// Bar count every series of the running evaluation must have.
pub(crate) fn bars() -> usize {
    BUDGET.with(Cell::get).map_or(0, |b| b.bars)
}

/// Run `f` with an evaluator limited to `timeout`, the tick, heap, call
/// stack and memory caps, for a chart of `bars` bars. Panics become errors.
fn limited<'v, 'a, R>(
    module: &'a Module<'v>,
    timeout: Duration,
    bars: usize,
    f: impl FnOnce(&mut Evaluator<'v, 'a, '_>) -> starlark::Result<R>,
) -> Result<R, String> {
    let budget = Budget {
        deadline: Instant::now() + timeout,
        mem_base: super::alloc::thread_net(),
        bars,
    };
    BUDGET.with(|c| c.set(Some(budget)));
    STOPPED.with(|c| c.set(None));
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut eval = Evaluator::new(module);
        eval.set_max_tick_count(MAX_TICKS)
            .and_then(|_| eval.set_max_heap_size(MAX_HEAP))
            .and_then(|_| eval.set_max_callstack_size(MAX_CALLSTACK))
            .map_err(|e| e.to_string())?;
        eval.set_check_cancelled(Box::new(move || {
            let stop = over(&budget);
            STOPPED.with(|c| c.set(stop));
            stop.is_some()
        }));
        f(&mut eval).map_err(|e| describe(&e))
    }));
    BUDGET.with(|c| c.set(None));
    match out {
        Ok(r) => r,
        Err(_) => Err("internal error while evaluating the script".into()),
    }
}

/// `message (line N)` → split later by `error_from`; a cancellation says
/// which limit stopped it.
fn describe(e: &starlark::Error) -> String {
    let line = error_line(e);
    let mut msg = e.without_diagnostic().to_string();
    if let Some(stop) = STOPPED.with(Cell::get).filter(|_| msg.contains("ancel")) {
        msg = stop.message();
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

/// Length of a list or dict that `_append` / `_insert` / `_setitem` may
/// grow; `None` for other values.
fn growable_len(v: Value) -> Option<usize> {
    if let Some(l) = ListRef::from_value(v) {
        return Some(l.len());
    }
    DictRef::from_value(v).map(|d| d.len())
}

fn too_big<T>(what: &str) -> starlark::Result<T> {
    err(format!("{what} too large (limit {MAX_SEQ} entries)"))
}

#[starlark_module]
fn guards(builder: &mut GlobalsBuilder) {
    /// `str(x)`, refusing results over `MAX_SEQ` characters.
    fn str<'v>(
        #[starlark(require = pos)] x: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        check()?;
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
        check()?;
        repr_len(x, MAX_SEQ)?;
        Ok(heap.alloc(x.to_repr()))
    }

    /// `int(x)` / `int(s, base)` limited to 64-bit results: scripts never
    /// see arbitrary-precision ints, whose arithmetic is unbounded in cost.
    fn int(
        #[starlark(require = pos)] x: Value,
        #[starlark(require = pos)] base: Option<i32>,
    ) -> starlark::Result<i64> {
        let out_of_range = || err("int() result out of the 64-bit range");
        if let Some(base) = base {
            let Some(s) = x.unpack_str() else {
                return err("int() with a base needs a string");
            };
            if !(2..=36).contains(&base) {
                return err("int() base must be in 2..36");
            }
            return i64::from_str_radix(s.trim(), base as u32).or_else(|_| out_of_range());
        }
        if let Some(b) = x.unpack_bool() {
            return Ok(i64::from(b));
        }
        if let Some(s) = x.unpack_str() {
            return s
                .trim()
                .parse::<i64>()
                .or_else(|_| err(format!("int(): invalid 64-bit integer {s:?}")));
        }
        if x.get_type() == "int" {
            return i64::unpack_value(x)
                .ok()
                .flatten()
                .map_or_else(out_of_range, Ok);
        }
        match UnpackFloat::unpack_value_opt(x).map(|f| f.0.trunc()) {
            Some(f) if f.is_finite() && f.abs() < 9.2e18 => Ok(f as i64),
            Some(_) => out_of_range(),
            None => err(format!("int() of a `{}`", x.get_type())),
        }
    }

    /// `l.append(x)`, after a size check (the static pass rewrites calls).
    fn _append<'v>(
        #[starlark(require = pos)] list: Value<'v>,
        #[starlark(require = pos)] x: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        check()?;
        if growable_len(list).is_some_and(|n| n >= MAX_SEQ) {
            return too_big("list");
        }
        let m = list.get_attr_error("append", eval.heap())?;
        eval.eval_function(m, &[x], &[])
    }

    /// `l.insert(i, x)`, after a size check.
    fn _insert<'v>(
        #[starlark(require = pos)] list: Value<'v>,
        #[starlark(require = pos)] i: Value<'v>,
        #[starlark(require = pos)] x: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        check()?;
        if growable_len(list).is_some_and(|n| n >= MAX_SEQ) {
            return too_big("list");
        }
        let m = list.get_attr_error("insert", eval.heap())?;
        eval.eval_function(m, &[i, x], &[])
    }

    /// `obj[k] = v`, refusing to grow a dict past `MAX_SEQ` entries.
    fn _setitem<'v>(
        #[starlark(require = pos)] obj: Value<'v>,
        #[starlark(require = pos)] k: Value<'v>,
        #[starlark(require = pos)] v: Value<'v>,
    ) -> starlark::Result<NoneType> {
        check()?;
        if let Some(d) = DictRef::from_value(obj) {
            if d.len() >= MAX_SEQ && d.get(k)?.is_none() {
                return too_big("dict");
            }
        }
        obj.set_at(k, v)?;
        Ok(NoneType)
    }

    /// `range(stop)` / `range(start, stop[, step])`, at most `MAX_SEQ` long.
    fn range(
        #[starlark(require = pos)] a: i32,
        #[starlark(require = pos)] b: Option<i32>,
        #[starlark(require = pos, default = 1)] step: i32,
    ) -> starlark::Result<Range> {
        check()?;
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
        check()?;
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
    /// Memory the frozen module holds (counted for the compile cache).
    pub bytes: usize,
}

fn parse(name: &str, source: &str) -> Result<AstModule, ScriptError> {
    AstModule::parse(&format!("{name}.star"), source.to_owned(), &dialect()).map_err(|e| {
        let line = e.span().map(|s| s.resolve_span().begin.line + 1);
        ScriptError::new("parse", line, e.without_diagnostic().to_string())
    })
}

/// Static check over the AST; returns the byte-offset source edits that
/// route augmented assignment and list/dict growth through the guarded
/// natives (see `walk`).
fn static_check(ast: &AstModule, source: &str) -> Result<Vec<Edit>, ScriptError> {
    let reserved: std::collections::HashSet<String> =
        globals().names().map(|n| n.as_str().to_owned()).collect();
    let fail = |span: Span, msg: String| {
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

    let mut problem: Option<(Span, String)> = None;
    let mut edits = Vec::new();
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

type Edit = (usize, usize, String);

fn pos(p: starlark::codemap::Pos) -> usize {
    p.get() as usize
}

/// Whether `e` is recognisably a series: an `ohlcv` column, a `ta.*` call,
/// or arithmetic, comparison or indexing on one. Variables are not tracked.
fn seriesish(e: &AstExpr) -> bool {
    match &e.node {
        ExprP::Dot(obj, attr) => {
            matches!(&obj.node, ExprP::Identifier(id) if id.ident == "ohlcv")
                && super::chart_builder::COLUMNS.contains(&attr.node.as_str())
        }
        ExprP::Call(callee, _) => matches!(&callee.node,
            ExprP::Dot(obj, f) if f.node != "nan"
                && matches!(&obj.node, ExprP::Identifier(id) if id.ident == "ta")),
        ExprP::Index(b) => seriesish(&b.0),
        ExprP::Op(l, op, r) => {
            !matches!(op, BinOp::And | BinOp::Or | BinOp::In | BinOp::NotIn)
                && (seriesish(l) || seriesish(r))
        }
        ExprP::Minus(x) | ExprP::Plus(x) | ExprP::BitNot(x) => seriesish(x),
        _ => false,
    }
}

const NOT_A_BOOL: &str = "a series is not a bool (it is always true); use chart.dot(cond, ...) for per-bar conditions or loop over series.list()";

fn index_target(t: &AstAssignTarget) -> bool {
    match &t.node {
        AssignTargetP::Index(_) => true,
        AssignTargetP::Tuple(xs) => xs.iter().any(index_target),
        _ => false,
    }
}

/// Static check of one AST node and its children. Records the first
/// problem, and the source edits that route growth and augmented
/// assignment through the guarded natives:
/// `x op= y` → `x = x op (y)`, `a.append(x)` → `_append(a, x)`,
/// `a.insert(i, x)` → `_insert(a, i, x)`, `d[k] = v` → `_setitem(d, k, (v))`.
fn walk(
    v: Visit<'_, AstNoPayload>,
    problem: &mut Option<(Span, String)>,
    edits: &mut Vec<Edit>,
    source: &str,
) {
    if problem.is_some() {
        return;
    }
    let mut flag = |span: Span, msg: &str| *problem = Some((span, msg.to_owned()));
    match &v {
        Visit::Stmt(s) => match &s.node {
            Stmt::Load(_) => flag(s.span, "load() is not allowed"),
            Stmt::If(c, _) | Stmt::IfElse(c, _) if seriesish(c) => flag(c.span, NOT_A_BOOL),
            Stmt::For(f) if index_target(&f.var) => {
                flag(f.var.span, "a for loop cannot assign to an index")
            }
            Stmt::AssignModify(lhs, op, rhs) => {
                if !matches!(lhs.node, AssignTargetP::Identifier(_)) {
                    flag(
                        lhs.span,
                        "augmented assignment (`+=` etc.) needs a plain name; write `x[i] = x[i] + v`",
                    );
                    return;
                }
                let sym = op.to_string();
                let sym = sym.trim().trim_end_matches('=');
                let (l0, l1) = (pos(lhs.span.begin()), pos(lhs.span.end()));
                let (r0, r1) = (pos(rhs.span.begin()), pos(rhs.span.end()));
                let lhs_src = &source[l0..l1];
                edits.push((l1, r0, format!(" = {lhs_src} {sym} (")));
                edits.push((r1, r1, ")".into()));
            }
            Stmt::Assign(a) => match &a.lhs.node {
                AssignTargetP::Index(ix) => {
                    let (obj, key) = (&ix.0, &ix.1);
                    let start = pos(a.lhs.span.begin());
                    edits.push((start, start, "_setitem(".into()));
                    edits.push((pos(obj.span.end()), pos(key.span.begin()), ", ".into()));
                    edits.push((pos(key.span.end()), pos(a.rhs.span.begin()), ", (".into()));
                    let end = pos(a.rhs.span.end());
                    edits.push((end, end, "))".into()));
                }
                AssignTargetP::Tuple(_) if index_target(&a.lhs) => flag(
                    a.lhs.span,
                    "assign list or dict items one at a time: `d[k] = v`",
                ),
                _ => {}
            },
            _ => {}
        },
        Visit::Expr(e) => match &e.node {
            ExprP::Dot(_, attr) if BANNED_ATTRS.contains(&attr.node.as_str()) => flag(
                e.span,
                &format!("`.{}()` is not available in scripts", attr.node),
            ),
            ExprP::Identifier(id) if BANNED_IDENTS.contains(&id.ident.as_str()) => flag(
                e.span,
                &format!("`{}` is not available in scripts", id.ident),
            ),
            ExprP::Literal(AstLiteral::Int(i)) => {
                if let TokenInt::BigInt(b) = &i.node {
                    if i64::try_from(b).is_err() {
                        flag(e.span, "integer literal out of the 64-bit range");
                    }
                }
            }
            ExprP::If(b) if seriesish(&b.0) => flag(b.0.span, NOT_A_BOOL),
            ExprP::Not(x) if seriesish(x) => flag(x.span, NOT_A_BOOL),
            ExprP::Op(l, BinOp::And | BinOp::Or, r) if seriesish(l) || seriesish(r) => {
                flag(e.span, NOT_A_BOOL)
            }
            ExprP::ListComprehension(_, first, rest) | ExprP::DictComprehension(_, first, rest) => {
                let mut fors =
                    std::iter::once(&**first).chain(rest.iter().filter_map(|c| match c {
                        ClauseP::For(f) => Some(f),
                        ClauseP::If(_) => None,
                    }));
                if fors.any(|f| index_target(&f.var)) {
                    flag(e.span, "a comprehension cannot assign to an index");
                } else if rest
                    .iter()
                    .any(|c| matches!(c, ClauseP::If(x) if seriesish(x)))
                {
                    flag(e.span, NOT_A_BOOL);
                }
            }
            ExprP::Call(callee, args) => {
                if let ExprP::Dot(obj, name) = &callee.node {
                    let f = match name.node.as_str() {
                        "append" => Some("_append("),
                        "insert" => Some("_insert("),
                        _ => None,
                    };
                    if let Some(f) = f {
                        let call_end = pos(e.span.end());
                        let args_begin = args
                            .args
                            .first()
                            .map_or(call_end - 1, |a| pos(a.span.begin()));
                        let start = pos(obj.span.begin());
                        edits.push((start, start, f.into()));
                        edits.push((pos(obj.span.end()), args_begin, ", ".into()));
                    }
                }
            }
            _ => {}
        },
    }
    v.visit_children(|c| walk(c, problem, edits, source));
}

/// Apply `(start, end, replacement)` byte edits (non-overlapping). At one
/// position, edits recorded first (outer nodes) end up leftmost.
fn apply_edits(source: &str, edits: Vec<Edit>) -> String {
    let mut edits: Vec<(usize, Edit)> = edits.into_iter().enumerate().collect();
    edits.sort_by_key(|(seq, e)| std::cmp::Reverse((e.0, e.1, *seq)));
    let mut out = source.to_owned();
    for (_, (a, b, text)) in edits {
        out.replace_range(a..b, &text);
    }
    out
}

/// Parse, static-check and evaluate the module top level, then check that
/// `PARAMS`, `META` and `compute` are present and well-formed.
pub fn compile(name: &str, source: &str) -> Result<Compiled, ScriptError> {
    compile_within(name, source, DRY_RUN_TIMEOUT)
}

/// `compile` with its limit on top-level code as an argument, so a test can
/// reach a guard behind that code on a loaded machine.
pub(super) fn compile_within(
    name: &str,
    source: &str,
    timeout: Duration,
) -> Result<Compiled, ScriptError> {
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

    let before = super::alloc::thread_net();
    let module = Module::with_temp_heap(|module| {
        limited(&module, timeout, 0, |eval| {
            eval.eval_module(ast, globals()).map(|_| ())
        })
        .map_err(|e| error_from("static", e))?;
        module
            .freeze()
            .map_err(|e| ScriptError::new("static", None, format!("{e:?}")))
    })?;
    // What the frozen module keeps alive once the temporary heap is gone.
    let bytes = (super::alloc::thread_net() - before).max(0) as usize;
    if bytes > MAX_MODULE_BYTES {
        return Err(ScriptError::new(
            "static",
            None,
            format!(
                "top-level values take {} KiB (limit {} KiB); build large data inside compute()",
                bytes >> 10,
                MAX_MODULE_BYTES >> 10
            ),
        ));
    }
    let info = presence(name, &module)?;
    Ok(Compiled {
        module,
        info,
        bytes,
    })
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
    // `name` is the entry key on the wire. The other entry keys (`id`,
    // `hidden`, `style`) are only rejected at registration (`scripting::validate`),
    // so scripts stored before they were reserved keep compiling.
    if name == "name" {
        return Err(format!("a param cannot be called \"{name}\" (reserved)"));
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
        limited(&module, timeout, data.len(), |eval| {
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

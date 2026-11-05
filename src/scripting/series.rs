//! The `Series` script value and the operator overloads (ADR-0024).
//!
//! A `Series` is one f64 per bar, aligned 1:1 with the chart bars; NaN is
//! Pine's `na`. Boolean series (comparisons, `ta.crossover`, ...) hold 1.0 /
//! 0.0, and a comparison with an `na` operand is false, as in Pine.
//!
//! starlark-rust lets a custom value overload `+`, `*` (both operand orders),
//! `-`, `/`, `&`, `|`, `~`, unary `-` and indexing, but `-` and `/` only for a
//! series on the left, and comparisons must return an `Ordering`. So the
//! runtime rewrites the binary operators `+ - * / % < <= > >= == != <<` into
//! calls to the `_op_*` functions below (`AstModule::replace_binary_operators`).
//! They give series math in both operand orders, comparisons that return a
//! boolean series, and size / overflow guards for the plain Starlark values.

use allocative::Allocative;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::dict::DictRef;
use starlark::values::float::UnpackFloat;
use starlark::values::list::{AllocList, ListRef};
use starlark::values::tuple::TupleRef;
use starlark::values::{
    starlark_value, Heap, NoSerialize, ProvidesStaticType, StarlarkPagablePanic, StarlarkValue,
    UnpackValue, Value,
};
use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use super::runtime::{charge, MAX_SEQ};

/// One value per bar. `boolean` marks a condition series (values 1.0 / 0.0).
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, StarlarkPagablePanic)]
pub struct Series {
    /// Shared so `ohlcv.close` and friends hand out a column without copying.
    pub v: Arc<[f64]>,
    pub boolean: bool,
}
starlark_simple_value!(Series);

impl fmt::Display for Series {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = if self.boolean {
            "bool series"
        } else {
            "series"
        };
        match self.v.last() {
            Some(last) => write!(f, "<{kind} len={} last={last}>", self.v.len()),
            None => write!(f, "<{kind} len=0>"),
        }
    }
}

/// Allocate a numeric series, charged against the evaluation's budget.
pub fn alloc<'v>(heap: Heap<'v>, v: Vec<f64>) -> starlark::Result<Value<'v>> {
    alloc_kind(heap, v, false)
}

/// Allocate a boolean series (values must be 1.0 / 0.0).
pub fn alloc_bool<'v>(heap: Heap<'v>, v: Vec<f64>) -> starlark::Result<Value<'v>> {
    alloc_kind(heap, v, true)
}

fn alloc_kind<'v>(heap: Heap<'v>, v: Vec<f64>, boolean: bool) -> starlark::Result<Value<'v>> {
    charge(v.len())?;
    Ok(heap.alloc(Series {
        v: v.into(),
        boolean,
    }))
}

pub fn err<T>(msg: impl Into<String>) -> starlark::Result<T> {
    Err(starlark::Error::new_other(anyhow::anyhow!(msg.into())))
}

fn truth(x: f64) -> bool {
    x != 0.0 && !x.is_nan()
}

fn b2f(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}

/// A series or a number (int, float or bool) in a series expression.
#[derive(Clone, Copy)]
pub enum Operand<'v> {
    S(&'v [f64]),
    N(f64),
}

impl<'v> Operand<'v> {
    pub fn of(v: Value<'v>) -> Option<Self> {
        if let Some(s) = Series::from_value(v) {
            return Some(Operand::S(&s.v));
        }
        if let Some(b) = v.unpack_bool() {
            return Some(Operand::N(b2f(b)));
        }
        UnpackFloat::unpack_value_opt(v).map(|f| Operand::N(f.0))
    }

    fn at(&self, i: usize) -> f64 {
        match self {
            Operand::S(s) => s[i],
            Operand::N(n) => *n,
        }
    }
}

/// Unpack an operand or fail with a type error naming `what`.
pub fn operand<'v>(v: Value<'v>, what: &str) -> starlark::Result<Operand<'v>> {
    Operand::of(v).ok_or_else(|| {
        starlark::Error::new_other(anyhow::anyhow!(
            "{what}: expected a series or a number, got `{}`",
            v.get_type()
        ))
    })
}

/// Common length of the operands; at least one must be a series.
pub fn common_len(ops: &[Operand]) -> starlark::Result<usize> {
    let mut len = None;
    for op in ops {
        if let Operand::S(s) = op {
            match len {
                None => len = Some(s.len()),
                Some(l) if l != s.len() => {
                    return err(format!("series length mismatch: {l} vs {}", s.len()))
                }
                _ => {}
            }
        }
    }
    len.map_or_else(|| err("expected at least one series"), Ok)
}

/// Element-wise `f(a, b)` over two operands, at least one a series.
pub fn zip_with(a: Operand, b: Operand, f: impl Fn(f64, f64) -> f64) -> starlark::Result<Vec<f64>> {
    let n = common_len(&[a, b])?;
    Ok((0..n).map(|i| f(a.at(i), b.at(i))).collect())
}

/// Element-wise `f(x)` over a series or a number; a number stays a number.
pub fn map_value<'v>(
    v: Value<'v>,
    heap: Heap<'v>,
    f: fn(f64) -> f64,
) -> starlark::Result<Value<'v>> {
    match operand(v, "argument")? {
        Operand::S(s) => alloc(heap, s.iter().map(|&x| f(x)).collect()),
        Operand::N(n) => Ok(heap.alloc(f(n))),
    }
}

/// Pine division: a zero denominator is na, never inf.
pub fn pine_div(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        f64::NAN
    } else {
        x / y
    }
}

#[derive(Clone, Copy)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl Cmp {
    fn ordering(self, o: Ordering) -> bool {
        match self {
            Cmp::Lt => o == Ordering::Less,
            Cmp::Le => o != Ordering::Greater,
            Cmp::Gt => o == Ordering::Greater,
            Cmp::Ge => o != Ordering::Less,
            Cmp::Eq => o == Ordering::Equal,
            Cmp::Ne => o != Ordering::Equal,
        }
    }

    /// Pine comparison: false when either side is na.
    fn floats(self, a: f64, b: f64) -> f64 {
        match a.partial_cmp(&b) {
            Some(o) => b2f(self.ordering(o)),
            None => 0.0,
        }
    }
}

fn is_series(v: Value) -> bool {
    Series::from_value(v).is_some()
}

fn series_arith<'v>(
    a: Value<'v>,
    b: Value<'v>,
    heap: Heap<'v>,
    what: &str,
    f: fn(f64, f64) -> f64,
) -> starlark::Result<Value<'v>> {
    let v = zip_with(operand(a, what)?, operand(b, what)?, f)?;
    alloc(heap, v)
}

fn compare<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>, c: Cmp) -> starlark::Result<Value<'v>> {
    if is_series(a) || is_series(b) {
        let v = zip_with(
            operand(a, "comparison")?,
            operand(b, "comparison")?,
            |x, y| c.floats(x, y),
        )?;
        return alloc_bool(heap, v);
    }
    let r = match c {
        Cmp::Eq => a.equals(b)?,
        Cmp::Ne => !a.equals(b)?,
        _ => c.ordering(a.compare(b)?),
    };
    Ok(Value::new_bool(r))
}

/// Integer operands for the overflow-checked int paths (`None` otherwise).
fn ints(a: Value, b: Value) -> Option<(i64, i64)> {
    let int = |v: Value| {
        if v.get_type() == "int" {
            i64::unpack_value(v).ok().flatten()
        } else {
            None
        }
    };
    Some((int(a)?, int(b)?))
}

fn checked<'v>(r: Option<i64>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
    match r {
        Some(v) => Ok(heap.alloc(v)),
        None => err("integer overflow (scripts use 64-bit integers)"),
    }
}

/// Length of a string, list or tuple; `None` for anything else.
fn seq_len(v: Value) -> Option<usize> {
    if let Some(s) = v.unpack_str() {
        return Some(s.len());
    }
    if let Some(l) = ListRef::from_value(v) {
        return Some(l.len());
    }
    TupleRef::from_value(v).map(|t| t.len())
}

fn too_long<T>(n: usize) -> starlark::Result<T> {
    err(format!(
        "result too large: {n} elements (limit {MAX_SEQ} per string, list or tuple)"
    ))
}

#[starlark_module]
pub fn operators(builder: &mut GlobalsBuilder) {
    fn _op_add<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        if is_series(a) || is_series(b) {
            return series_arith(a, b, heap, "+", |x, y| x + y);
        }
        if let Some((x, y)) = ints(a, b) {
            return checked(x.checked_add(y), heap);
        }
        if let (Some(x), Some(y)) = (seq_len(a), seq_len(b)) {
            if x + y > MAX_SEQ {
                return too_long(x + y);
            }
        }
        a.add(b, heap)
    }

    fn _op_sub<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        if is_series(a) || is_series(b) {
            return series_arith(a, b, heap, "-", |x, y| x - y);
        }
        if let Some((x, y)) = ints(a, b) {
            return checked(x.checked_sub(y), heap);
        }
        a.sub(b, heap)
    }

    fn _op_mul<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        if is_series(a) || is_series(b) {
            return series_arith(a, b, heap, "*", |x, y| x * y);
        }
        if let Some((x, y)) = ints(a, b) {
            return checked(x.checked_mul(y), heap);
        }
        // Repetition (`[0] * n`, `"ab" * n`): check the size before allocating.
        for (seq, n) in [(a, b), (b, a)] {
            if let (Some(len), Some(n)) = (seq_len(seq), i64::unpack_value(n).ok().flatten()) {
                let total = (len as u128) * (n.max(0) as u128);
                if total > MAX_SEQ as u128 {
                    return too_long(total.min(usize::MAX as u128) as usize);
                }
            }
        }
        a.mul(b, heap)
    }

    fn _op_div<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        if is_series(a) || is_series(b) {
            return series_arith(a, b, heap, "/", pine_div);
        }
        a.div(b, heap)
    }

    fn _op_mod<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        if let Some(fmt) = a.unpack_str() {
            // `"%s" % x`: bound the output by the format plus its arguments.
            if DictRef::from_value(b).is_some() {
                return err("`%` formatting with a dict is not supported in scripts");
            }
            let total = fmt.len() + super::runtime::repr_len(b, MAX_SEQ)?;
            if total > MAX_SEQ {
                return too_long(total);
            }
        }
        a.percent(b, heap)
    }

    fn _op_lshift<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        match ints(a, b) {
            Some((x, s)) if (0..63).contains(&s) => checked(x.checked_mul(1i64 << s), heap),
            Some(_) => err("shift count must be in 0..62"),
            None => a.left_shift(b, heap),
        }
    }

    fn _op_lt<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        compare(a, b, heap, Cmp::Lt)
    }

    fn _op_le<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        compare(a, b, heap, Cmp::Le)
    }

    fn _op_gt<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        compare(a, b, heap, Cmp::Gt)
    }

    fn _op_ge<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        compare(a, b, heap, Cmp::Ge)
    }

    fn _op_eq<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        compare(a, b, heap, Cmp::Eq)
    }

    fn _op_ne<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        compare(a, b, heap, Cmp::Ne)
    }
}

/// Operator → `_op_*` function, for `AstModule::replace_binary_operators`.
pub const OPERATOR_FUNCTIONS: &[(&str, &str)] = &[
    ("+", "_op_add"),
    ("-", "_op_sub"),
    ("*", "_op_mul"),
    ("/", "_op_div"),
    ("%", "_op_mod"),
    ("<<", "_op_lshift"),
    ("<", "_op_lt"),
    ("<=", "_op_le"),
    (">", "_op_gt"),
    (">=", "_op_ge"),
    ("==", "_op_eq"),
    ("!=", "_op_ne"),
];

fn logic<'v>(
    me: &Series,
    other: Value<'v>,
    heap: Heap<'v>,
    f: fn(bool, bool) -> bool,
) -> starlark::Result<Value<'v>> {
    let v = zip_with(
        Operand::S(&me.v),
        operand(other, "logical operator")?,
        |x, y| b2f(f(truth(x), truth(y))),
    )?;
    alloc_bool(heap, v)
}

#[starlark_value(type = "series")]
impl<'v> StarlarkValue<'v> for Series {
    fn get_methods() -> Option<&'static Methods> {
        starlark::methods_static!(METHODS = series_methods);
        Some(METHODS.methods())
    }

    fn length(&self) -> starlark::Result<i32> {
        Ok(self.v.len() as i32)
    }

    /// `s[k]`: the series `k` bars ago (Pine's history operator).
    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        match index.unpack_i32() {
            Some(k) if k >= 0 => alloc_kind(
                heap,
                crate::indicators::pine::shift(&self.v, k as i64),
                self.boolean,
            ),
            _ => err("series[k]: k must be an int >= 0 (bars ago); use .get(i) for one value"),
        }
    }

    fn minus(&self, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        alloc(heap, self.v.iter().map(|x| -x).collect())
    }

    fn plus(&self, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        alloc_kind(heap, self.v.to_vec(), self.boolean)
    }

    fn bit_and(&self, other: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        logic(self, other, heap, |a, b| a && b)
    }

    fn bit_or(&self, other: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        logic(self, other, heap, |a, b| a || b)
    }

    fn bit_xor(&self, other: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        logic(self, other, heap, |a, b| a != b)
    }

    fn bit_not(&self, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        alloc_bool(heap, self.v.iter().map(|&x| b2f(!truth(x))).collect())
    }
}

#[starlark_module]
fn series_methods(builder: &mut MethodsBuilder) {
    /// The value at bar `i` (negative counts from the last bar), as a float.
    fn get(this: &Series, i: i32) -> starlark::Result<f64> {
        let n = this.v.len() as i64;
        let idx = if i < 0 { n + i as i64 } else { i as i64 };
        if (0..n).contains(&idx) {
            Ok(this.v[idx as usize])
        } else {
            err(format!("series.get({i}): index out of range for {n} bars"))
        }
    }

    /// All values as a list of floats (na is `nan`), for per-bar loops.
    fn list<'v>(this: &Series, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        charge(this.v.len())?;
        Ok(heap.alloc(AllocList(this.v.iter().copied())))
    }
}

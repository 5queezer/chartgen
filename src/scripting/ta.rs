//! The `ta` namespace: Rust-backed, vectorized Pine `ta.*` functions bound
//! to `indicators::pine` (Pine v5 semantics, ADR-0018), plus element-wise
//! series math.

use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::float::UnpackFloat;
use starlark::values::list::UnpackList;
use starlark::values::tuple::AllocTuple;
use starlark::values::{Heap, Value};

use super::chart_builder::Ohlcv;
use super::runtime::check;
use super::series::{
    alloc, alloc_bool, common_len, err, map_value, operand, pine_div, zip_with, Operand, Series,
};
use crate::indicators::pine;

/// Longest accepted window / lookback.
const MAX_LENGTH: i32 = 10_000;

fn length(n: i32, what: &str) -> starlark::Result<usize> {
    if (1..=MAX_LENGTH).contains(&n) {
        Ok(n as usize)
    } else {
        err(format!(
            "{what}: length must be in 1..{MAX_LENGTH}, got {n}"
        ))
    }
}

/// `f(src)` as a new series; the deadline is checked before the work, as
/// windowed helpers cost O(bars * length).
fn out<'v>(
    heap: Heap<'v>,
    src: &Series,
    f: impl FnOnce(&[f64]) -> Vec<f64>,
) -> starlark::Result<Value<'v>> {
    check()?;
    alloc(heap, f(&src.v))
}

fn values(op: Operand, n: usize) -> Vec<f64> {
    match op {
        Operand::S(s) => s.to_vec(),
        Operand::N(x) => vec![x; n],
    }
}

fn bools(v: Vec<bool>) -> Vec<f64> {
    v.into_iter().map(|b| if b { 1.0 } else { 0.0 }).collect()
}

fn binary<'v>(
    a: Value<'v>,
    b: Value<'v>,
    heap: Heap<'v>,
    what: &str,
    f: fn(f64, f64) -> f64,
) -> starlark::Result<Value<'v>> {
    let (x, y) = (operand(a, what)?, operand(b, what)?);
    if let (Operand::N(x), Operand::N(y)) = (x, y) {
        return Ok(heap.alloc(f(x, y)));
    }
    alloc(heap, zip_with(x, y, f)?)
}

fn cross<'v>(
    a: Value<'v>,
    b: Value<'v>,
    heap: Heap<'v>,
    f: fn(&[f64], &[f64]) -> Vec<bool>,
) -> starlark::Result<Value<'v>> {
    let (x, y) = (operand(a, "cross")?, operand(b, "cross")?);
    let n = common_len(&[x, y])?;
    alloc_bool(heap, bools(f(&values(x, n), &values(y, n))))
}

#[starlark_module]
pub fn ta_module(builder: &mut GlobalsBuilder) {
    /// Pine `na`: a float NaN.
    const nan: f64 = f64::NAN;

    /// A series from a list of numbers (`None` is na).
    fn series<'v>(values: UnpackList<Value<'v>>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let v = values
            .items
            .iter()
            .map(|x| {
                if x.is_none() {
                    Ok(f64::NAN)
                } else {
                    match operand(*x, "ta.series") {
                        Ok(Operand::N(n)) => Ok(n),
                        _ => err("ta.series: list items must be numbers or None"),
                    }
                }
            })
            .collect::<starlark::Result<Vec<f64>>>()?;
        alloc(heap, v)
    }

    fn sma<'v>(src: &Series, length: i32, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.sma")?;
        out(heap, src, |x| pine::sma_n(x, n))
    }

    fn ema<'v>(src: &Series, length: i32, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.ema")?;
        out(heap, src, |x| pine::ema_n(x, n))
    }

    fn wma<'v>(src: &Series, length: i32, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.wma")?;
        out(heap, src, |x| pine::wma_n(x, n))
    }

    fn rma<'v>(src: &Series, length: i32, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.rma")?;
        out(heap, src, |x| pine::rma_n(x, n))
    }

    fn rsi<'v>(src: &Series, length: i32, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.rsi")?;
        out(heap, src, |x| pine::rsi(x, n))
    }

    /// Pine `ta.stoch(src, high, low, length)`.
    fn stoch<'v>(
        src: &Series,
        high: &Series,
        low: &Series,
        length: i32,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.stoch")?;
        common_len(&[Operand::S(&src.v), Operand::S(&high.v), Operand::S(&low.v)])?;
        out(heap, src, |x| pine::stoch(x, &high.v, &low.v, n))
    }

    /// Pine `ta.tr(true)`: true range of the chart bars.
    fn tr<'v>(ohlcv: &Ohlcv, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        alloc(heap, pine::true_range(&ohlcv.data))
    }

    /// Pine `ta.atr(length)` over the chart bars.
    fn atr<'v>(ohlcv: &Ohlcv, length: i32, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.atr")?;
        alloc(heap, pine::atr(&ohlcv.data, n))
    }

    /// Pine `ta.macd(src, fast, slow, signal)` → `(macd, signal, hist)`.
    fn macd<'v>(
        src: &Series,
        #[starlark(default = 12)] fast: i32,
        #[starlark(default = 26)] slow: i32,
        #[starlark(default = 9)] signal: i32,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let (f, s, g) = (
            self::length(fast, "ta.macd")?,
            self::length(slow, "ta.macd")?,
            self::length(signal, "ta.macd")?,
        );
        check()?;
        let line: Vec<f64> = pine::ema_n(&src.v, f)
            .iter()
            .zip(pine::ema_n(&src.v, s))
            .map(|(a, b)| a - b)
            .collect();
        let sig = pine::ema_n(&line, g);
        let hist: Vec<f64> = line.iter().zip(&sig).map(|(a, b)| a - b).collect();
        let parts = [alloc(heap, line)?, alloc(heap, sig)?, alloc(heap, hist)?];
        Ok(heap.alloc(AllocTuple(parts)))
    }

    fn highest<'v>(src: &Series, length: i32, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.highest")?;
        out(heap, src, |x| pine::highest(x, n))
    }

    fn lowest<'v>(src: &Series, length: i32, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.lowest")?;
        out(heap, src, |x| pine::lowest(x, n))
    }

    /// Population standard deviation (Pine `ta.stdev`, biased).
    fn stdev<'v>(src: &Series, length: i32, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.stdev")?;
        out(heap, src, |x| pine::stdev(x, n))
    }

    /// `src - src[length]`.
    fn change<'v>(
        src: &Series,
        #[starlark(default = 1)] length: i32,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.change")?;
        out(heap, src, |x| pine::change_n(x, n))
    }

    /// `100 * (src - src[length]) / src[length]`.
    fn roc<'v>(src: &Series, length: i32, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let n = self::length(length, "ta.roc")?;
        out(heap, src, |x| pine::roc(x, n))
    }

    fn crossover<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        cross(a, b, heap, pine::crossover)
    }

    fn crossunder<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        cross(a, b, heap, pine::crossunder)
    }

    /// Pine `ta.pivothigh(src, left, right)`.
    fn pivot_high<'v>(
        src: &Series,
        left: i32,
        right: i32,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let (l, r) = (
            self::length(left, "ta.pivot_high")?,
            self::length(right, "ta.pivot_high")?,
        );
        out(heap, src, |x| pine::pivot_high(x, l, r))
    }

    /// Pine `ta.pivotlow(src, left, right)`.
    fn pivot_low<'v>(
        src: &Series,
        left: i32,
        right: i32,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let (l, r) = (
            self::length(left, "ta.pivot_low")?,
            self::length(right, "ta.pivot_low")?,
        );
        out(heap, src, |x| pine::pivot_low(x, l, r))
    }

    /// Pine `nz(src, replacement)`: na → `replacement`.
    fn nz<'v>(
        src: &Series,
        #[starlark(default = UnpackFloat(0.0))] replacement: UnpackFloat,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let r = replacement.0;
        out(heap, src, |x| {
            x.iter().map(|&v| if v.is_nan() { r } else { v }).collect()
        })
    }

    /// Pine `na(src)` as a bool series.
    fn na<'v>(src: &Series, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        alloc_bool(
            heap,
            src.v
                .iter()
                .map(|v| if v.is_nan() { 1.0 } else { 0.0 })
                .collect(),
        )
    }

    /// Pine `fixnan`: na → the last non-na value.
    fn fixnan<'v>(src: &Series, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        out(heap, src, pine::fixnan)
    }

    // Element-wise math; every argument is a series or a number. The
    // `_scalar` names match the proposal and accept the same arguments.

    fn add<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        binary(a, b, heap, "ta.add", |x, y| x + y)
    }
    fn sub<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        binary(a, b, heap, "ta.sub", |x, y| x - y)
    }
    fn mul<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        binary(a, b, heap, "ta.mul", |x, y| x * y)
    }
    /// Division; a zero denominator is na.
    fn div<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        binary(a, b, heap, "ta.div", pine_div)
    }
    fn add_scalar<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        binary(a, b, heap, "ta.add_scalar", |x, y| x + y)
    }
    fn sub_scalar<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        binary(a, b, heap, "ta.sub_scalar", |x, y| x - y)
    }
    fn mul_scalar<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        binary(a, b, heap, "ta.mul_scalar", |x, y| x * y)
    }
    fn div_scalar<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        binary(a, b, heap, "ta.div_scalar", pine_div)
    }
    /// Pine `math.max`: na if either side is na.
    fn max<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        binary(a, b, heap, "ta.max", pine::nan_max)
    }
    /// Pine `math.min`: na if either side is na.
    fn min<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        binary(a, b, heap, "ta.min", pine::nan_min)
    }
    fn pow<'v>(a: Value<'v>, b: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        binary(a, b, heap, "ta.pow", f64::powf)
    }
    fn abs<'v>(x: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        map_value(x, heap, f64::abs)
    }
    fn sqrt<'v>(x: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        map_value(x, heap, f64::sqrt)
    }
    fn log<'v>(x: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        map_value(x, heap, f64::ln)
    }
    fn exp<'v>(x: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        map_value(x, heap, f64::exp)
    }

    // Price sources, as in the proposal's `ta.hlc3(ohlcv)`; same as `ohlcv.hlc3`.

    fn hl2<'v>(ohlcv: &Ohlcv, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        alloc(heap, pine::source_series(&ohlcv.data, "hl2"))
    }
    fn hlc3<'v>(ohlcv: &Ohlcv, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        alloc(heap, pine::source_series(&ohlcv.data, "hlc3"))
    }
    fn ohlc4<'v>(ohlcv: &Ohlcv, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        alloc(heap, pine::source_series(&ohlcv.data, "ohlc4"))
    }
}

/// OHLCV bar and data container.

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Bar {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub date: String,
}

#[derive(Clone, Debug)]
pub struct OhlcvData {
    pub bars: Vec<Bar>,
}

impl OhlcvData {
    pub fn closes(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.close).collect()
    }
    pub fn highs(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.high).collect()
    }
    pub fn lows(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.low).collect()
    }
    pub fn len(&self) -> usize {
        self.bars.len()
    }
}

/// Simple LCG PRNG (no external dep needed).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_f64(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / (u32::MAX as f64)
    }
    /// Approximate normal via Box-Muller.
    fn normal(&mut self, mean: f64, std: f64) -> f64 {
        let u1 = self.next_f64().max(1e-10);
        let u2 = self.next_f64();
        mean + std * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

pub fn sample_data(n: usize) -> OhlcvData {
    let mut rng = Rng::new(42);
    let mut price = 30.0_f64;
    let mut bars = Vec::with_capacity(n);

    for i in 0..n {
        let o = price + rng.normal(0.0, 0.5);
        let c = o + rng.normal(-0.1, 1.0);
        let h = o.max(c) + rng.normal(0.0, 0.5).abs();
        let l = o.min(c) - rng.normal(0.0, 0.5).abs();
        let v = rng.normal(10.0, 0.8).exp();
        bars.push(Bar {
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
            date: format!("D{}", i),
        });
        price = c;
    }
    OhlcvData { bars }
}

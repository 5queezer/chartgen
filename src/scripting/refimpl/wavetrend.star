# WaveTrend oscillator: reference implementation of the built-in
# `wavetrend` (src/indicators/wavetrend.rs). The parity test in
# src/scripting/tests.rs requires the same output within 1e-9, so change
# both in the same PR. Fork it with `fork_indicator("wavetrend")`.

PARAMS = [
    {"name": "ch_len", "type": "int", "default": 9, "min": 1, "max": 10000, "desc": "Channel length"},
    {"name": "avg_len", "type": "int", "default": 12, "min": 1, "max": 10000, "desc": "Average length"},
]

META = {
    "title": "WaveTrend",
    "description": "WaveTrend Oscillator — cycle-based momentum",
    "overlay": False,
}

MA_LEN = 3
OB = 53.0
OS = -53.0

def compute(ohlcv, params, chart):
    ap = ohlcv.hlc3
    esa = ta.ema(ap, params["ch_len"])
    d = ta.ema(ta.abs(ap - esa), params["ch_len"])
    ci = (ap - esa) / (0.015 * d)  # a zero d is na (Pine division)
    wt1 = ta.ema(ci, params["avg_len"])
    wt2 = ta.sma(wt1, MA_LEN)

    chart.line(wt1, title = "WT1", color = "#4994ec", alpha = 0.8, width = 2)
    chart.line(wt2, title = "WT2", color = "#1f1559", alpha = 0.8, width = 2)
    chart.fill(wt1, wt2, color = "#4994ec", alpha = 0.25)
    chart.fill(wt1 - wt2, 0, color = "#ffffff", alpha = 0.15)

    # Crosses as in the built-in: wt1 was below wt2 and is now at or above
    # it (and the mirror); na on either bar is no cross.
    up = (wt1[1] < wt2[1]) & (wt1 >= wt2)
    down = (wt1[1] > wt2[1]) & (wt1 <= wt2)
    oversold = wt2 <= OS
    overbought = wt2 >= OB
    chart.dot(up & oversold, wt2, color = "#3fff00", size = 6, label = "cross_up_os")
    chart.dot(down & overbought, wt2, color = "#ff0000", size = 6, label = "cross_down_ob")
    chart.dot(up & ~oversold, wt2, color = "#00e676", alpha = 0.7, size = 3, label = "cross_up")
    chart.dot(down & ~overbought, wt2, color = "#ff5252", alpha = 0.7, size = 3, label = "cross_down")

    chart.hline(OB, color = "#ffffff", alpha = 0.15)
    chart.hline(OS, color = "#ffffff", alpha = 0.15)
    chart.hline(0, color = "#787b86", alpha = 0.3)
    chart.set_y_range(-105, 105)

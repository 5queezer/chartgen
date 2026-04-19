#!/usr/bin/env bash
# Renders one chart per indicator into website/src/assets/indicators/.
# Run from repo root: ./scripts/gen_screenshots.sh
set -euo pipefail

OUTPUT_DIR="website/src/assets/indicators"
BIN="target/release/chartgen"

# Use equity data for overlays and panels, crypto for external/derivatives.
# AAPL on 1d gives clean stock-market visuals; BTCUSDT exercises the Binance
# endpoints the external indicators actually target.
SYMBOL_EQUITY="AAPL"
SYMBOL_CRYPTO="BTCUSDT"
INTERVAL="1d"
BARS="200"
WIDTH="1280"
HEIGHT="640"

OVERLAY=(
    ema_stack bbands keltner donchian vwap vwap_bands supertrend sar
    ichimoku heikin_ashi pivot volume_profile session_vp hvn_lvn naked_poc tpo
)

PANEL=(
    cipher_b macd rsi wavetrend stoch atr obv cci roc mfi williams_r cmf
    adx ad histvol kalman_volume rsi_mfi_stoch
)

EXTERNAL=(cvd funding oi long_short fear_greed)

echo "Building chartgen (release)..."
cargo build --release

mkdir -p "$OUTPUT_DIR"

render() {
    local name="$1"
    local symbol="$2"
    local out="$OUTPUT_DIR/$name.png"
    printf "  %-20s %s -> %s ... " "$name" "$symbol" "$out"
    if "$BIN" -s "$symbol" -i "$INTERVAL" -p "$name" \
        -n "$BARS" --width "$WIDTH" --height "$HEIGHT" \
        -o "$out" >/dev/null 2>&1; then
        echo "ok"
    else
        echo "FAILED"
    fi
}

echo
echo "Rendering overlays..."
for n in "${OVERLAY[@]}"; do render "$n" "$SYMBOL_EQUITY"; done

echo
echo "Rendering panels..."
for n in "${PANEL[@]}"; do render "$n" "$SYMBOL_EQUITY"; done

echo
echo "Rendering external (crypto)..."
for n in "${EXTERNAL[@]}"; do render "$n" "$SYMBOL_CRYPTO"; done

echo
echo "Done. Output in $OUTPUT_DIR/"
ls -1 "$OUTPUT_DIR" | wc -l | awk '{print $1 " PNGs written"}'

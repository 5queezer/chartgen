// AUTO-GENERATED FROM chartgen tools/list — DO NOT EDIT.
// Regenerate with `./scripts/gen-mcp-types.sh`.
//
// Source: Rust `chartgen::mcp_schema::tools_list_result()` via
// `examples/gen_mcp_types.rs`.
// Drift is enforced by CI (`mcp-types-drift` job).

/* eslint-disable */

export interface CancelAlertInput {
  /**
   * Alert ID to cancel
   */
  alert_id: string;
}

export interface CancelOrderInput {
  /**
   * Order ID to cancel
   */
  order_id: string;
}

export interface GenerateChartInput {
  /**
   * Number of OHLCV candlestick bars to display
   */
  bars?: number;
  /**
   * Output format. 'png' (default) returns a rendered chart image. 'summary' returns a compact JSON summary (OHLCV stats, last indicator values, signals, divergences, levels) — typically 10-50x fewer tokens than a PNG and ideal for automated analysis without vision. 'both' returns the PNG plus the JSON summary. 'series' returns a full per-bar time-series JSON payload (every OHLCV bar plus 1:1 aligned indicator value arrays with NaN→null for pre-warmup, signals with bar timestamps, hlines, and y_range) — ideal for downstream numeric computation.
   */
  format?: "png" | "summary" | "both" | "series";
  /**
   * Chart image height in pixels
   */
  height?: number;
  /**
   * Technical indicators to render on the chart. Each element is a name string (e.g. 'rsi') or an object with 'name' and optional parameters (e.g. {"name": "rsi", "length": 21}). Overlays draw on the price chart, panels draw below. Call list_indicators for the full list of 33 available indicators.
   */
  indicators?: (
    | string
    | {
        name: string;
        [k: string]: unknown;
      }
  )[];
  /**
   * Alias for timeframe.
   */
  interval?: string;
  /**
   * Alias for indicators.
   */
  panels?: (
    | string
    | {
        name: string;
        [k: string]: unknown;
      }
  )[];
  /**
   * Alias for ticker. Stock or crypto symbol.
   */
  symbol?: string;
  /**
   * Stock or crypto ticker symbol. Stocks: AAPL, MSFT, TSLA, GOOGL, AMZN (Yahoo Finance). Crypto: BTCUSDT, ETHUSDT, SOLUSDT (Binance). If omitted, uses random sample data.
   */
  ticker?: string;
  /**
   * Candlestick timeframe / interval: 1m, 5m, 15m, 1h, 4h, 1d, 1wk
   */
  timeframe?: string;
  /**
   * Chart image width in pixels
   */
  width?: number;
}

export interface GetBalanceInput {}

export interface GetIndicatorsInput {
  /**
   * List of indicator names to compute (e.g. ["rsi", "macd", "adx"])
   */
  indicators: string[];
  /**
   * Candle interval (e.g. 1m, 5m, 15m, 1h, 4h, 1d)
   */
  interval?: string;
  /**
   * Trading symbol (e.g. BTCUSDT, ETHUSDT, AAPL)
   */
  symbol: string;
}

export interface GetNotificationsInput {}

export interface GetOrdersInput {
  /**
   * Filter: 'open' for open orders only, 'all' for all orders
   */
  filter?: "open" | "all";
}

export interface GetPositionsInput {}

export interface ListAlertsInput {}

export interface ListIndicatorsInput {}

export interface ListSubscriptionsInput {}

export interface PlaceOrderInput {
  /**
   * Limit price (required for limit orders)
   */
  price?: number;
  /**
   * Order quantity
   */
  quantity: number;
  /**
   * Order side
   */
  side: "buy" | "sell";
  /**
   * Trading pair (e.g. BTCUSDT)
   */
  symbol: string;
  /**
   * Order type
   */
  type: "market" | "limit";
}

export interface SetAlertInput {
  /**
   * Alert condition. One of: {"price_above": 0.85}, {"price_below": 0.805}, or {"indicator_signal": {"indicator": "cipher_b", "signal": "green_dot"}}
   */
  condition: {};
  /**
   * Trading pair (e.g. BTCUSDT)
   */
  symbol: string;
}

export interface SubscribeNotificationsInput {
  /**
   * Filter by alert types: price_above, price_below, indicator_signal. Omit for all.
   */
  alert_types?: string[];
  /**
   * Filter by trading pairs (e.g. ["BTCUSDT"]). Omit for all.
   */
  symbols?: string[];
}

export interface UnsubscribeNotificationsInput {}

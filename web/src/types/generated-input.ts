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

export interface CreateServerAlertInput {
  /**
   * crossing: the price goes from one side of the level to the other, or onto it. crossing_up / crossing_down: only that way. greater / less: the price is above / below the level. A crossing needs a previous reading, so the first price the alert sees never fires one.
   */
  condition: "crossing" | "crossing_up" | "crossing_down" | "greater" | "less";
  /**
   * live: every price update, like a browser alert. close: only when a bar of the interval closes.
   */
  evaluate_on?: "live" | "close";
  /**
   * Bar interval, e.g. 1m, 15m, 1h, 1d. With evaluate_on: close the alert reads the closes of these bars, which need a Binance kline stream; with live it only labels the alert.
   */
  interval?: string;
  /**
   * The price level; greater than 0.
   */
  level: number;
  /**
   * Your note, sent in the webhook payload.
   */
  message?: string;
  /**
   * Notify your devices (subscribe_push) when it fires. May go with a webhook.
   */
  push?: boolean;
  /**
   * Sign deliveries with a per-alert secret (HMAC-SHA256). Needs webhook_url.
   */
  sign?: boolean;
  /**
   * Binance symbol, e.g. BTCUSDT.
   */
  symbol: string;
  /**
   * once: switch off after firing. every: stay active; greater and less wait until the price is back on the other side.
   */
  trigger: "once" | "every";
  /**
   * https URL to POST to when the alert fires.
   */
  webhook_url?: string;
}

export interface DeleteServerAlertInput {
  /**
   * The alert's id.
   */
  alert_id: string;
}

export interface GenerateChartInput {
  /**
   * Number of OHLCV candlestick bars to display (max 5000, all formats)
   */
  bars?: number;
  /**
   * Inputs of a price-based chart_style, refused for the others. Echoed in format=series and summary with the box_size the build used. Send that box_size back on later polls and pages to keep the same bars. subscribe_chart requires it for an atr or percent box, so pushed bars line up with the chart that was loaded.
   */
  chart_params?: {
    /**
     * ATR length, 1 to 500.
     */
    atr_length?: number;
    /**
     * How the box is set, for renko, kagi (the reversal amount), point_figure and range (the range size). 'atr' (default): ATR of the closed source bars. 'fixed': box_size. 'percent': a percentage of the last price. ATR and percent are rounded to two significant digits.
     */
    box?: "atr" | "fixed" | "percent";
    /**
     * The size of a fixed box, or the resolved size of an atr or percent box to keep. A size that would make over 20,000 bars is refused.
     */
    box_size?: number;
    /**
     * line_break: how many lines a close must pass to reverse, 1 to 20.
     */
    lines?: number;
    /**
     * Percentage of the last price, above 0 and at most 100.
     */
    percent?: number;
    /**
     * point_figure: boxes the close must come back to start a new column, 1 to 20.
     */
    reversal?: number;
  };
  /**
   * Price series style. 'candles' (default) uses the raw OHLC bars. 'heikin_ashi' replaces them with Heikin Ashi bars (haClose = ohlc4, haOpen = (prev haOpen + prev haClose) / 2, seeded with (open+close)/2) in every format. Like TradingView, indicators are then computed on the Heikin Ashi values. The transform runs over bars+warmup before trimming, so pages and tail polls converge after warmup. 'renko', 'kagi', 'point_figure', 'line_break' and 'range' are price-based: built from the bars of `timeframe`, so they only approximate tick data, with times that are not evenly spaced. The last bar is the open one; only closed source bars confirm a bar. Each reads a fixed window of source bars (up to 5000, within the 20,000 native bar limit), `warmup` does nothing for them, and a window may hold fewer bars than `bars`. Indicators run on these bars; ones that project past the last bar return no `future`. format=series echoes the applied style as chart_style. See ADR-0047.
   */
  chart_style?: "candles" | "heikin_ashi" | "renko" | "kagi" | "point_figure" | "line_break" | "range";
  /**
   * History paging: return only bars whose open time (ms since epoch) is strictly before end_time — pass the oldest loaded bar's t × 1000 to fetch the page before it. Omit (or null) for the newest bars, including the still-forming one. Must be an integer between -2208988800000 (1900-01-01) and one year from now.
   */
  end_time?: number;
  /**
   * Output format. 'png' (default) returns a rendered chart image. 'summary' returns a compact JSON summary (OHLCV stats, last indicator values, signals, divergences, levels) — typically 10-50x fewer tokens than a PNG and ideal for automated analysis without vision. 'both' returns the PNG plus the JSON summary. 'series' returns a full per-bar time-series JSON payload (every OHLCV bar plus 1:1 aligned indicator value arrays with NaN→null for pre-warmup, signals with bar timestamps, hlines, and y_range, a per-indicator future block with the values plotted past the last bar (Ichimoku cloud), plus has_more=true when older history may exist) — ideal for downstream numeric computation.
   */
  format?: "png" | "summary" | "both" | "series";
  /**
   * Chart image height in pixels
   */
  height?: number;
  /**
   * Technical indicators to render on the chart. Each element is a name string (e.g. 'rsi') or an object with 'name' and optional parameters (e.g. {"name": "rsi", "length": 21}). Results are keyed by name; give an object an 'id' to key it by that instead. The same indicator may appear several times (e.g. RSI 14 and RSI 21): a repeat without an id is keyed name#2, name#3, and so on; an id must not equal the name of an entry without one. At most 50 entries. Overlays draw on the price chart, panels draw below. Call list_indicators for the full list of 33 available indicators.
   */
  indicators?: (
    | string
    | {
        /**
         * Instance key (ADR-0028): the response keys this entry by id instead of name, so one indicator can be requested several times with different parameters.
         */
        id?: string;
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
        /**
         * Instance key (ADR-0028): the response keys this entry by id instead of name, so one indicator can be requested several times with different parameters.
         */
        id?: string;
        name: string;
        [k: string]: unknown;
      }
  )[];
  /**
   * Alias for ticker. Stock or crypto symbol.
   */
  symbol?: string;
  /**
   * Stock or crypto ticker symbol. Stocks: AAPL, MSFT, TSLA, GOOGL, AMZN (Yahoo Finance). Crypto: BTCUSDT, ETHUSDT, SOLUSDT (Binance). If omitted, uses random sample data. If given and the upstream fetch fails, the call returns an error.
   */
  ticker?: string;
  /**
   * Candlestick timeframe / interval: a number and a unit, s seconds, m minutes, h hours, D days, W weeks, M months (1s, 5s, 1m, 5m, 7m, 45m, 90m, 4h, 8h, 1d, 2D, 1W, 2W, 1M). Seconds need a Binance spot symbol: Yahoo has none. Upper bounds after normalization: under 60s, 1440m, 168h, 365D, 52W, 12M. A minute or hour interval may also take at most 60 native Binance bars per bar, so 61m is rejected and 65m is not. The response echoes the canonical form (120m becomes 2h, 2W becomes 2wk, 1M becomes 1mo).
   */
  timeframe?: string;
  /**
   * format=series only: extra older bars (max 1000) fetched and fed to indicator computation, then trimmed, so the first returned bars carry warmed-up indicator values. A non-zero warmup is raised to what the indicators need to converge (891 for ema_stack's EMA 200). Ignored by png/summary/both.
   */
  warmup?: number;
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
   * Candle interval, as generate_chart's timeframe (e.g. 1m, 7m, 45m, 1h, 4h, 8h, 1d, 2D, 2W)
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

export interface GetQuotesInput {
  /**
   * Ticker symbols (e.g. ["BTCUSDT", "AAPL", "^GSPC"]). At most 50 entries; trimmed, uppercased and deduplicated.
   *
   * @maxItems 50
   */
  symbols: string[];
}

export interface ListAlertsInput {}

export interface ListIndicatorsInput {}

export interface ListServerAlertsInput {}

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

export interface SearchSymbolsInput {
  /**
   * Maximum number of results
   */
  limit?: number;
  /**
   * Text to search for in tickers and names. Trimmed; 1-40 characters, no control characters.
   */
  query: string;
  /**
   * Only return these instrument types. Omit for all types.
   */
  types?: ("crypto" | "stock" | "etf" | "fund" | "index" | "forex" | "future")[];
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

export interface SubscribeChartInput {
  /**
   * Inputs of a price-based chart_style, refused for the others. Echoed in format=series and summary with the box_size the build used. Send that box_size back on later polls and pages to keep the same bars. subscribe_chart requires it for an atr or percent box, so pushed bars line up with the chart that was loaded.
   */
  chart_params?: {
    /**
     * ATR length, 1 to 500.
     */
    atr_length?: number;
    /**
     * How the box is set, for renko, kagi (the reversal amount), point_figure and range (the range size). 'atr' (default): ATR of the closed source bars. 'fixed': box_size. 'percent': a percentage of the last price. ATR and percent are rounded to two significant digits.
     */
    box?: "atr" | "fixed" | "percent";
    /**
     * The size of a fixed box, or the resolved size of an atr or percent box to keep. A size that would make over 20,000 bars is refused.
     */
    box_size?: number;
    /**
     * line_break: how many lines a close must pass to reverse, 1 to 20.
     */
    lines?: number;
    /**
     * Percentage of the last price, above 0 and at most 100.
     */
    percent?: number;
    /**
     * point_figure: boxes the close must come back to start a new column, 1 to 20.
     */
    reversal?: number;
  };
  /**
   * Price series style, as for generate_chart. Price-based tails carry up to 64 bars, as one closed source bar can confirm several.
   */
  chart_style?: "candles" | "heikin_ashi" | "renko" | "kagi" | "point_figure" | "line_break" | "range";
  /**
   * Indicators, as for generate_chart; at most 10.
   */
  indicators?: (
    | string
    | {
        /**
         * Instance key (ADR-0028): the response keys this entry by id instead of name, so one indicator can be requested several times with different parameters.
         */
        id?: string;
        name: string;
        [k: string]: unknown;
      }
  )[];
  /**
   * Binance symbol, 2-20 letters or digits, e.g. BTCUSDT.
   */
  ticker: string;
  /**
   * Candlestick timeframe, as for generate_chart: a number and a unit (1s, 1m, 7m, 45m, 4h, 8h, 1d, 2D, 2W, 1M). Binance pushes intervals it has no stream for by resampling a finer one.
   */
  timeframe?: string;
  /**
   * Extra older bars (max 1000) the server keeps and feeds to indicator computation, as for generate_chart(format=series), and raised the same way.
   */
  warmup?: number;
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

export interface SubscribePushInput {
  /**
   * The subscription's auth secret, base64url.
   */
  auth: string;
  /**
   * PushSubscription.endpoint.
   */
  endpoint: string;
  /**
   * The subscription's p256dh key, base64url.
   */
  p256dh: string;
}

export interface UnsubscribeChartInput {
  /**
   * The subscription_id returned by subscribe_chart.
   */
  subscription_id: string;
}

export interface UnsubscribeNotificationsInput {}

export interface UnsubscribePushInput {
  /**
   * PushSubscription.endpoint.
   */
  endpoint: string;
}

export interface UpdateServerAlertInput {
  /**
   * Pause (false) or resume (true).
   */
  active?: boolean;
  /**
   * The alert's id.
   */
  alert_id: string;
  /**
   * crossing: the price goes from one side of the level to the other, or onto it. crossing_up / crossing_down: only that way. greater / less: the price is above / below the level. A crossing needs a previous reading, so the first price the alert sees never fires one.
   */
  condition?: "crossing" | "crossing_up" | "crossing_down" | "greater" | "less";
  /**
   * The price level; greater than 0.
   */
  level?: number;
  /**
   * Your note; an empty string removes it.
   */
  message?: string;
  /**
   * Notify your devices when it fires, or stop.
   */
  push?: boolean;
  /**
   * true: sign with a new secret. false: stop signing.
   */
  sign?: boolean;
  /**
   * once: switch off after firing. every: stay active; greater and less wait until the price is back on the other side.
   */
  trigger?: "once" | "every";
  /**
   * New https URL, or an empty string to remove the webhook.
   */
  webhook_url?: string;
}

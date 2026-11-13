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

export interface CancelSetupInput {
  /**
   * Replay servers only (ADR-0059), and required there: the time this call is answered for, as unix seconds, unix ms or an ISO 8601 time. Only bars that closed at or before it are used, and it cannot be earlier than the latest as_of the server has been sent, by any caller. On any other server leave it out.
   */
  as_of?: number | string;
  /**
   * The setup's id.
   */
  id: string;
  /**
   * Why this change, in plain text. Kept in the setup's log.
   */
  reason: string;
}

export interface CreateServerAlertInput {
  /**
   * level: the line against level. close: the bar's close against the line, with no level; a crossing is then the close and the line one bar before against both now.
   */
  compare?: "level" | "close";
  /**
   * crossing: the price goes from one side of the level to the other, or onto it. crossing_up / crossing_down: only that way. greater / less: the price is above / below the level. A crossing needs a previous reading, so the first price the alert sees never fires one. On an indicator alert the line is what crosses the level, or with compare: close the close crosses the line, and the previous reading is the bar before's. bar_close: a bar of the interval closed, whatever the price; it needs an interval of 1h or longer, takes no level, indicator or compare, and an alert cannot change into or out of it.
   */
  condition: "crossing" | "crossing_up" | "crossing_down" | "greater" | "less" | "bar_close";
  /**
   * live: every price update, like a browser alert. close: only when a bar of the interval closes.
   */
  evaluate_on?: "live" | "close";
  /**
   * Watch this indicator instead of the price, at bar closes: needs evaluate_on: close and an interval of 1h or longer. For an EMA or SMA of a length use ma with params {type: "ema", length: 50}. Fixed after creation.
   */
  indicator?: "rsi" | "ma" | "macd" | "stoch" | "cci" | "mfi" | "williams_r" | "roc" | "atr" | "adx";
  /**
   * Bar interval, e.g. 1m, 15m, 1h, 1d. With evaluate_on: close the alert reads the closes of these bars, which need a Binance kline stream; with live it only labels the alert. Required with condition bar_close, 1h or longer.
   */
  interval?: string;
  /**
   * The price level, greater than 0. With indicator, the level the line is compared with, any number; none with compare: close or condition: bar_close.
   */
  level?: number;
  /**
   * Which of the indicator's lines, by its legend label, e.g. Signal for macd or %D for stoch. Default: its first line.
   */
  line?: string;
  /**
   * Your note, sent in the webhook payload.
   */
  message?: string;
  /**
   * Wake your agent when it fires: a signed POST to the receiver this server's operator configured for you. Instead of webhook_url, not with it. Refused where no receiver is configured for you.
   */
  notify_agent?: boolean;
  /**
   * The indicator's params, as list_indicators lists them, e.g. {length: 14}. A name it does not take is refused; the alert keeps every param with its default filled in. Its history must fit in 999 bars (EMA 200 does); stoch, cci, mfi, williams_r and roc take at most 200 per param; offset is not taken.
   */
  params?: {
    [k: string]: unknown;
  };
  /**
   * Notify your devices (subscribe_push) when it fires. May go with a webhook.
   */
  push?: boolean;
  /**
   * One of your open setups this alert is about. Needs notify_agent: the wake-up then carries the setup as it stands (status, levels, fill, result, shadow results, thesis).
   */
  setup_id?: string;
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

export interface CreateSetupInput {
  /**
   * Replay servers only (ADR-0059), and required there: the time this call is answered for, as unix seconds, unix ms or an ISO 8601 time. Only bars that closed at or before it are used, and it cannot be earlier than the latest as_of the server has been sent, by any caller. On any other server leave it out.
   */
  as_of?: number | string;
  /**
   * take: a setup with your levels. skip: a decision not to trade, scored with the server's default bracket; send no entry, invalidation, targets or expires_at with it.
   */
  decision?: "take" | "skip";
  direction: "long" | "short";
  /**
   * Drawings that show the setup, exactly as generate_chart's `drawings` takes them (ADR-0054) and checked by the same rules against the bars of the setup's symbol and interval. Stored with their resolved points.
   *
   * @maxItems 50
   */
  drawings?: {}[];
  /**
   * The price zone the setup is entered in. The same price twice is a level.
   */
  entry?: {
    /**
     * Upper edge; at least low.
     */
    high: number;
    /**
     * Lower edge; greater than 0.
     */
    low: number;
  };
  /**
   * When the setup runs out, as unix seconds, unix ms or an ISO 8601 time; in the future and at most 90 days ahead.
   */
  expires_at?: number | string;
  /**
   * Optional. What you measured when you made this setup, as a flat object of at most 16 entries: names of a-z, 0-9 and _, values a number, true or false, or a string of at most 32 characters. The server gives them no meaning. Fixed once the setup is made; get_track_record can group by one of them.
   */
  features?: {
    [k: string]: number | boolean | string;
  };
  /**
   * The chart interval the setup was read on, e.g. 15m, 1h, 4h. Its drawings are placed on these bars.
   */
  interval: string;
  /**
   * The price at which the setup is wrong: below the entry zone of a long, above that of a short, and at least 3 times the assumed round-trip cost away from it.
   */
  invalidation?: number;
  /**
   * Optional. A label of at most 32 characters for the rules you followed, so that results can be told apart by it. Fixed once the setup is made.
   */
  playbook_version?: string;
  /**
   * Optional. Your own estimate, from 0 to 1, that this setup reaches its first target. Fixed once the setup is made; get_track_record shows it beside what happened.
   */
  probability?: number;
  /**
   * Binance symbol, e.g. BTCUSDT.
   */
  symbol: string;
  /**
   * Your labels for the setup, to group and filter by, such as strategy:retest-short or trigger:100eyes:abnormal-volatility. At most 10, each 1 to 64 characters of lowercase letters, digits and : - _ . /, starting with a letter or digit. A tag given twice is kept once.
   *
   * @maxItems 10
   */
  tags?:
    | []
    | [string]
    | [string, string]
    | [string, string, string]
    | [string, string, string, string]
    | [string, string, string, string, string]
    | [string, string, string, string, string, string]
    | [string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string, string];
  /**
   * Target prices beyond the entry zone, nearest first. The first one decides a win; the others are kept for the chart.
   *
   * @minItems 1
   * @maxItems 4
   */
  targets?: [number] | [number, number] | [number, number, number] | [number, number, number, number];
  /**
   * Why, in plain text.
   */
  thesis: string;
}

export interface DeleteServerAlertInput {
  /**
   * The alert's id.
   */
  alert_id: string;
}

export interface GenerateChartInput {
  /**
   * Replay servers only (ADR-0059), and required there: the time this call is answered for, as unix seconds, unix ms or an ISO 8601 time. Only bars that closed at or before it are used, and it cannot be earlier than the latest as_of the server has been sent, by any caller. On any other server leave it out.
   */
  as_of?: number | string;
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
   * Drawings to put on the price chart, as a person draws them in the web app (ADR-0054). Each is {tool, points, style?, text?} with exactly the tool's points: segment (start; end), rayLine (origin; a point it passes through, after which it runs on to the edge), straightLine (a point on the line; another point on the line), horizontalStraightLine (the price level (its time only places the handle)), horizontalRayLine (origin; the ray runs right from it at its price), verticalStraightLine (the bar (its price only places the handle)), parallelStraightLine (first point of the base line; second point of the base line; a point the parallel line passes through), fibonacciLine (the 100% level; the 0% level), cgRect (a corner; the opposite corner), cgText (where the text starts (its left edge, vertically centred)), simpleTag (the price (its time only places the handle)), cgFibExt (A, where the move starts; B, where it ends; C, where the projection starts), cgPitchfork (P1, the handle; P2; P3), cgPriceRange (from; to), cgDateRange (from; to), cgDatePriceRange (from; to), cgLong (entry; its time is the box's left edge; target, above the entry; its time is the box's right edge; stop, below the entry; the same time as the target), cgShort (entry; its time is the box's left edge; target, below the entry; its time is the box's right edge; stop, above the entry; the same time as the target). A point is {time, price} or an anchor {time, snap} that takes that bar's open, high, low or close; time is unix seconds, unix ms or an ISO 8601 date or date-time (UTC unless it has an offset) and resolves to the bar containing it, or past the last bar to that bar's grid. All drawings are checked together: if any is invalid the call fails, names each bad drawing by index with a reason and a hint, and draws nothing. On success the answer adds a JSON text item: each drawing with its resolved points (time, timestamp in ms, price, bar), warnings (a point cut off past the right edge or outside the visible price range), and open_url, a link that opens the chart in the web app with these drawings as normal, editable drawings (when the server has CHARTGEN_BASE_URL). Call inspect_chart first to anchor points to real swing highs and lows; list_drawing_tools describes every tool. At most 50 drawings.
   *
   * @maxItems 50
   */
  drawings?: {
    /**
     * Exactly the tool's points, in its order (1 to 3).
     */
    points: {
      /**
       * The point's price. Give price or snap.
       */
      price?: number;
      /**
       * Anchor to this value of the bar at time, instead of a price. Only on loaded bars.
       */
      snap?: "open" | "high" | "low" | "close";
      /**
       * Unix seconds, unix ms, or ISO 8601 (2026-10-12 or 2026-10-12T14:00:00Z).
       */
      time: number | string;
    }[];
    /**
     * The look, as the web app's style bar sets it; absent fields take the tool's own. cgText takes color and fontSize; the others color, width and dash, and the filled ones (cgRect and the three ranges) fill.
     */
    style?: {
      /**
       * #rgb, #rrggbb, #rrggbbaa or rgba(r, g, b, a).
       */
      color?: string;
      dash?: "solid" | "dashed" | "dotted";
      /**
       * Fill color, as color; may carry alpha.
       */
      fill?: string;
      fontSize?: 10 | 12 | 14 | 18 | 24;
      /**
       * Line width in px.
       */
      width?: number;
    };
    /**
     * cgText's text (required there); no other tool takes text.
     */
    text?: string;
    tool:
      | "segment"
      | "rayLine"
      | "straightLine"
      | "horizontalStraightLine"
      | "horizontalRayLine"
      | "verticalStraightLine"
      | "parallelStraightLine"
      | "fibonacciLine"
      | "cgRect"
      | "cgText"
      | "simpleTag"
      | "cgFibExt"
      | "cgPitchfork"
      | "cgPriceRange"
      | "cgDateRange"
      | "cgDatePriceRange"
      | "cgLong"
      | "cgShort";
  }[];
  /**
   * History paging: return only bars whose open time (ms since epoch) is strictly before end_time — pass the oldest loaded bar's t × 1000 to fetch the page before it. Omit (or null) for the newest bars, including the still-forming one. Must be an integer between -2208988800000 (1900-01-01) and one year from now.
   */
  end_time?: number;
  /**
   * Output format. 'png' (default) returns a rendered chart image. 'summary' returns a compact JSON summary (OHLCV stats, last indicator values, signals, divergences, levels) — typically 10-50x fewer tokens than a PNG and ideal for automated analysis without vision. 'both' returns the PNG plus the JSON summary. 'series' returns a full per-bar time-series JSON payload (every OHLCV bar plus 1:1 aligned indicator value arrays with NaN→null for pre-warmup, signals with bar timestamps, hlines, and y_range, a per-indicator future block with the values plotted past the last bar (Ichimoku cloud), plus has_more=true when older history may exist) — ideal for downstream numeric computation.
   */
  format?: "png" | "summary" | "both" | "series";
  /**
   * Chart image height in pixels. See width.
   */
  height?: number;
  /**
   * Technical indicators to render on the chart. Each element is a name string (e.g. 'rsi') or an object with 'name' and optional parameters (e.g. {"name": "rsi", "length": 21}). Results are keyed by name; give an object an 'id' to key it by that instead. The same indicator may appear several times (e.g. RSI 14 and RSI 21): a repeat without an id is keyed name#2, name#3, and so on; an id must not equal the name of an entry without one. At most 50 entries. Overlays draw on the price chart, panels draw below. Call list_indicators for the full list of 79 available indicators.
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
   * format=series only: compute volume_profile, fixed_range_vp and anchored_vp from lower-timeframe bars, for a chart's full load: up to 6 more Binance requests. The series then says which bars in profile_interval. Leave it off on pages and tails, which keep the load's profile. Default: false, each bar's volume spread over its range.
   */
  profile_intrabars?: boolean;
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
   * format=series only: extra older bars (max 1000) fetched and fed to indicator computation, then trimmed, so the first returned bars carry warmed-up indicator values. A non-zero warmup is raised to what the indicators need to converge (891 for ema_stack's EMA 200). png/summary/both ignore it: with indicators and a ticker they always read what those need, at least 300 bars, and still show exactly bars (ADR-0056).
   */
  warmup?: number;
  /**
   * Chart image width in pixels. width x height counts towards what one call may hold: 4000 x 4000 fits with little on it, and a call past that is refused (-32602).
   */
  width?: number;
}

export interface GetBalanceInput {}

export interface GetFootprintInput {
  /**
   * ATR length of the auto row size.
   */
  atr_length?: number;
  /**
   * Newest bars to return.
   */
  bars?: number;
  /**
   * Exclusive upper bound on bar open time, ms since epoch.
   */
  end_time?: number;
  /**
   * Imbalance, percent: buy against sell one row below, sell against buy one row above.
   */
  imbalance?: number;
  /**
   * For a poll of a chart already loaded: the intrabar_interval its load returned, or 'none' if that was null, so the few newest bars are read as the load's were. Falls back to what these bars can afford if they cannot have it. Omit to choose by the window.
   */
  intrabar_interval?: string;
  /**
   * Row height in price. Raised to a tick, lowered to the highest price, and widened if a bar would have over 200 rows; row_widened says so. Omit for row_ticks or auto.
   */
  row_size?: number;
  /**
   * Row height in ticks. Omit for auto: 0.2 x ATR on round numbers.
   */
  row_ticks?: number;
  /**
   * Imbalances in a row that make a stacked imbalance. A level counts only where at least 2 intrabars traded, on it and on the level it is compared with: one kline has one split, so its rows would all agree.
   */
  stacked?: number;
  /**
   * Symbol, as generate_chart's ticker.
   */
  ticker: string;
  /**
   * Chart interval, as generate_chart's timeframe.
   */
  timeframe?: string;
  /**
   * Value area, percent of the volume.
   */
  value_area?: number;
}

export interface GetIndicatorsInput {
  /**
   * Replay servers only (ADR-0059), and required there: the time this call is answered for, as unix seconds, unix ms or an ISO 8601 time. Only bars that closed at or before it are used, and it cannot be earlier than the latest as_of the server has been sent, by any caller. On any other server leave it out.
   */
  as_of?: number | string;
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
   * Replay servers only (ADR-0059), and required there: the time this call is answered for, as unix seconds, unix ms or an ISO 8601 time. Only bars that closed at or before it are used, and it cannot be earlier than the latest as_of the server has been sent, by any caller. On any other server leave it out.
   */
  as_of?: number | string;
  /**
   * Ticker symbols (e.g. ["BTCUSDT", "AAPL", "^GSPC"]). At most 50 entries; trimmed, uppercased and deduplicated.
   *
   * @maxItems 50
   */
  symbols: string[];
}

export interface GetSetupInput {
  /**
   * Replay servers only (ADR-0059), and required there: the time this call is answered for, as unix seconds, unix ms or an ISO 8601 time. Only bars that closed at or before it are used, and it cannot be earlier than the latest as_of the server has been sent, by any caller. On any other server leave it out.
   */
  as_of?: number | string;
  /**
   * The setup's id.
   */
  id: string;
}

export interface GetTpoInput {
  /**
   * Length of a block, from 5m to 4h. If 3000 cannot hold a whole period, the next that can: the reply's block is that one, and block_widened says so. A period 4h blocks cannot cover is refused.
   */
  block?: string;
  /**
   * Exclusive upper bound on block open time, ms since epoch.
   */
  end_time?: number;
  /**
   * Blocks of the initial balance.
   */
  ib_blocks?: number;
  /**
   * Length of a profile: a count and D, W or M, such as 1D, 1W or 1M.
   */
  period?: string;
  /**
   * Newest periods to return, whole ones within 3000 blocks; `periods_cut` says if that left some out.
   */
  periods?: number;
  /**
   * Row height in price. Omit for row_ticks or auto.
   */
  row_size?: number;
  /**
   * Row height in ticks. Omit for auto: a 24th of the median period range, on round numbers.
   */
  row_ticks?: number;
  /**
   * Up to 5 HH:MM-HH:MM in the exchange's time (UTC for crypto): one profile per session of each period.
   *
   * @maxItems 5
   */
  sessions?:
    | []
    | [string]
    | [string, string]
    | [string, string, string]
    | [string, string, string, string]
    | [string, string, string, string, string];
  /**
   * Symbol, as generate_chart's ticker.
   */
  ticker: string;
  /**
   * Value area, percent of the blocks.
   */
  value_area?: number;
}

export interface GetTrackRecordInput {
  /**
   * Replay servers only (ADR-0059), and required there: the time this call is answered for, as unix seconds, unix ms or an ISO 8601 time. Only bars that closed at or before it are used, and it cannot be earlier than the latest as_of the server has been sent, by any caller. On any other server leave it out.
   */
  as_of?: number | string;
  /**
   * Only setups you took, or only decisions not to trade.
   */
  decision?: "take" | "skip";
  direction?: "long" | "short";
  /**
   * With group_by: feature, the name of the feature to group by.
   */
  feature?: string;
  /**
   * Only setups made at or after this time, as unix seconds, unix ms or an ISO 8601 time.
   */
  from?: number | string;
  /**
   * Count per value of one of these. probability groups by tenths (0.0, 0.1 ... 0.9, with 1 in the last). A setup that stated none is in the group whose key is null. More than 50 groups are refused.
   */
  group_by?: "playbook_version" | "feature" | "probability" | "symbol" | "direction";
  /**
   * Only setups made with this playbook_version.
   */
  playbook_version?: string;
  /**
   * Only setups on this symbol.
   */
  symbol?: string;
  /**
   * Only setups made before this time, as unix seconds, unix ms or an ISO 8601 time.
   */
  to?: number | string;
}

export interface InspectChartInput {
  /**
   * Replay servers only (ADR-0059), and required there: the time this call is answered for, as unix seconds, unix ms or an ISO 8601 time. Only bars that closed at or before it are used, and it cannot be earlier than the latest as_of the server has been sent, by any caller. On any other server leave it out.
   */
  as_of?: number | string;
  /**
   * Bars to read, newest last, 1 to 1000.
   */
  bars?: number;
  /**
   * As generate_chart's end_time: only bars that open before it (ms since epoch).
   */
  end_time?: number;
  /**
   * Alias for timeframe.
   */
  interval?: string;
  /**
   * Bars on each side a swing high or low must top (or undercut), 1 to 50. Larger finds fewer, bigger swings.
   */
  swing_depth?: number;
  /**
   * Alias for ticker.
   */
  symbol?: string;
  /**
   * Stock or crypto symbol, as generate_chart takes it. Required.
   */
  ticker?: string;
  /**
   * As generate_chart's timeframe.
   */
  timeframe?: string;
}

export interface ListAlertsInput {}

export interface ListDrawingToolsInput {}

export interface ListEventsInput {
  /**
   * Return events with a number above this one: the cursor of your last answer. 0 or absent: from the oldest kept.
   */
  after?: number;
  /**
   * Most events to return; more says whether others wait.
   */
  limit?: number;
}

export interface ListIndicatorsInput {}

export interface ListServerAlertsInput {}

export interface ListSetupsInput {
  /**
   * Replay servers only (ADR-0059), and required there: the time this call is answered for, as unix seconds, unix ms or an ISO 8601 time. Only bars that closed at or before it are used, and it cannot be earlier than the latest as_of the server has been sent, by any caller. On any other server leave it out.
   */
  as_of?: number | string;
  /**
   * Most setups to return; total says how many matched.
   */
  limit?: number;
  /**
   * open: watching or triggered. closed: any other. Omit for all.
   */
  status?: "open" | "closed" | "watching" | "triggered" | "won" | "invalidated" | "expired" | "cancelled";
  /**
   * Only this symbol's.
   */
  symbol?: string;
  /**
   * Only setups that have every one of these tags.
   *
   * @maxItems 10
   */
  tags?:
    | []
    | [string]
    | [string, string]
    | [string, string, string]
    | [string, string, string, string]
    | [string, string, string, string, string]
    | [string, string, string, string, string, string]
    | [string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string, string];
}

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

export interface ReviseSetupInput {
  /**
   * Replay servers only (ADR-0059), and required there: the time this call is answered for, as unix seconds, unix ms or an ISO 8601 time. Only bars that closed at or before it are used, and it cannot be earlier than the latest as_of the server has been sent, by any caller. On any other server leave it out.
   */
  as_of?: number | string;
  /**
   * Drawings that show the setup, exactly as generate_chart's `drawings` takes them (ADR-0054) and checked by the same rules against the bars of the setup's symbol and interval. Stored with their resolved points.
   *
   * @maxItems 50
   */
  drawings?: {}[];
  /**
   * The price zone the setup is entered in. The same price twice is a level.
   */
  entry?: {
    /**
     * Upper edge; at least low.
     */
    high: number;
    /**
     * Lower edge; greater than 0.
     */
    low: number;
  };
  /**
   * When the setup runs out, as unix seconds, unix ms or an ISO 8601 time; in the future and at most 90 days ahead.
   */
  expires_at?: number | string;
  /**
   * The setup's id.
   */
  id: string;
  /**
   * The price at which the setup is wrong: below the entry zone of a long, above that of a short, and at least 3 times the assumed round-trip cost away from it.
   */
  invalidation?: number;
  /**
   * Why this change, in plain text. Kept in the setup's log.
   */
  reason: string;
  /**
   * Your labels for the setup, to group and filter by, such as strategy:retest-short or trigger:100eyes:abnormal-volatility. At most 10, each 1 to 64 characters of lowercase letters, digits and : - _ . /, starting with a letter or digit. A tag given twice is kept once.
   *
   * @maxItems 10
   */
  tags?:
    | []
    | [string]
    | [string, string]
    | [string, string, string]
    | [string, string, string, string]
    | [string, string, string, string, string]
    | [string, string, string, string, string, string]
    | [string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string]
    | [string, string, string, string, string, string, string, string, string, string];
  /**
   * Target prices beyond the entry zone, nearest first. The first one decides a win; the others are kept for the chart.
   *
   * @minItems 1
   * @maxItems 4
   */
  targets?: [number] | [number, number] | [number, number, number] | [number, number, number, number];
  /**
   * Why, in plain text.
   */
  thesis?: string;
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
   * crossing: the price goes from one side of the level to the other, or onto it. crossing_up / crossing_down: only that way. greater / less: the price is above / below the level. A crossing needs a previous reading, so the first price the alert sees never fires one. On an indicator alert the line is what crosses the level, or with compare: close the close crosses the line, and the previous reading is the bar before's. bar_close: a bar of the interval closed, whatever the price; it needs an interval of 1h or longer, takes no level, indicator or compare, and an alert cannot change into or out of it.
   */
  condition?: "crossing" | "crossing_up" | "crossing_down" | "greater" | "less" | "bar_close";
  /**
   * The price level, greater than 0, or an indicator alert's level, any number. Not on an alert with compare: close or condition: bar_close.
   */
  level?: number;
  /**
   * Your note; an empty string removes it.
   */
  message?: string;
  /**
   * Wake your agent when it fires, or stop. Not together with a webhook_url.
   */
  notify_agent?: boolean;
  /**
   * Notify your devices when it fires, or stop.
   */
  push?: boolean;
  /**
   * One of your open setups this alert is about, or an empty string to take it off. Needs notify_agent.
   */
  setup_id?: string;
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

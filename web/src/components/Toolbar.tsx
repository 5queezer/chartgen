// Top toolbar: symbol button (opens the search dialog), timeframe
// buttons with an overflow menu, and the chart-type menu. Also owns the
// TradingView-style type-to-act shortcuts: a letter typed anywhere
// outside an input opens symbol search prefilled with it; a digit opens
// the timeframe-entry popup.

import { Dialog, DropdownMenu } from "@kobalte/core";
import {
  For,
  Show,
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";

import {
  FAVOURITE_TIMEFRAMES,
  TIMEFRAME_OPTIONS,
  parseTimeframe,
  timeframeGroup,
  timeframeLabel,
  type Timeframe,
} from "../lib/timeframe";
import { toggleFullscreen } from "../lib/chartActions";
import { isTypingContext } from "../lib/keys";
import {
  CHART_TYPES,
  chartType,
  setChartType,
  setSymbol,
  setTheme,
  setTimeframe as storeSetTimeframe,
  symbol,
  theme,
  timeframe,
  type ChartType,
} from "../stores/layout";
import type { McpSession } from "../mcp";
import {
  BP_FULL,
  BP_NARROW,
  BP_TIMEFRAMES,
  BP_TINY,
  compact,
  createViewportWidth,
} from "../lib/viewport";
import { SymbolSearch } from "./SymbolSearch";

const CHART_TYPE_LABELS: Record<ChartType, string> = {
  candles: "Candles",
  hollow: "Hollow candles",
  bars: "Bars",
  line: "Line",
  line_markers: "Line with markers",
  step_line: "Step line",
  area: "Area",
  baseline: "Baseline",
  columns: "Columns",
  high_low: "High-low",
  heikin_ashi: "Heikin Ashi",
};

function ChartTypeIcon(props: { type: ChartType }): JSX.Element {
  const common = {
    width: 18,
    height: 18,
    viewBox: "0 0 18 18",
    fill: "none",
    stroke: "currentColor",
    "stroke-width": 1.5,
    "aria-hidden": true,
  } as const;
  switch (props.type) {
    case "candles":
      return (
        <svg {...common}>
          <path d="M5 2v14M13 4v11" />
          <rect x="3" y="5" width="4" height="7" fill="currentColor" />
          <rect x="11" y="7" width="4" height="5" fill="currentColor" />
        </svg>
      );
    case "heikin_ashi":
      return (
        <svg {...common}>
          <path d="M5 1v16M13 3v13" />
          <rect x="3" y="4" width="4" height="9" fill="currentColor" />
          <rect x="11" y="6" width="4" height="6" fill="currentColor" />
          <path d="M1 15h16" stroke-dasharray="2 2" />
        </svg>
      );
    case "hollow":
      return (
        <svg {...common}>
          <path d="M5 2v14M13 4v11" />
          <rect x="3" y="5" width="4" height="7" fill="var(--color-bg)" />
          <rect x="11" y="7" width="4" height="5" fill="var(--color-bg)" />
        </svg>
      );
    case "bars":
      return (
        <svg {...common}>
          <path d="M5 2v14M3 5h2M5 12h2M13 4v11M11 7h2M13 12h2" />
        </svg>
      );
    case "line":
      return (
        <svg {...common}>
          <path d="M2 13l4-5 3 3 7-8" stroke-linejoin="round" />
        </svg>
      );
    case "area":
      return (
        <svg {...common}>
          <path
            d="M2 13l4-5 3 3 7-8v10H2z"
            fill="currentColor"
            fill-opacity="0.25"
            stroke-linejoin="round"
          />
        </svg>
      );
    case "line_markers":
      return (
        <svg {...common}>
          <path d="M2 13l4-5 3 3 7-8" stroke-linejoin="round" />
          <g fill="currentColor" stroke="none">
            <circle cx="2" cy="13" r="1.6" />
            <circle cx="6" cy="8" r="1.6" />
            <circle cx="9" cy="11" r="1.6" />
            <circle cx="16" cy="3" r="1.6" />
          </g>
        </svg>
      );
    case "step_line":
      return (
        <svg {...common}>
          <path d="M1 14h4V9h4v3h4V4h4" stroke-linejoin="round" />
        </svg>
      );
    case "baseline":
      return (
        <svg {...common}>
          <path d="M1 9h16" stroke-dasharray="1 2" stroke-width="1" />
          <path d="M2 6l4-3 3 4 3 5 4 2" stroke-linejoin="round" />
        </svg>
      );
    case "columns":
      return (
        <svg {...common} fill="currentColor" stroke="none">
          <rect x="2" y="9" width="3" height="7" />
          <rect x="7.5" y="5" width="3" height="11" />
          <rect x="13" y="2" width="3" height="14" />
        </svg>
      );
    case "high_low":
      return (
        <svg {...common} fill="currentColor" stroke="none">
          <rect x="2" y="6" width="3" height="8" />
          <rect x="7.5" y="2" width="3" height="9" />
          <rect x="13" y="5" width="3" height="11" />
        </svg>
      );
  }
}

const iconProps = {
  width: 18,
  height: 18,
  viewBox: "0 0 18 18",
  fill: "none",
  stroke: "currentColor",
  "stroke-width": 1.5,
  "aria-hidden": true,
} as const;

const CompareIcon = () => (
  <svg {...iconProps}>
    <circle cx="6.5" cy="9" r="4.5" />
    <circle cx="11.5" cy="9" r="4.5" />
  </svg>
);
const ReplayIcon = () => (
  <svg {...iconProps}>
    <path d="M9 4.5v9L3 9zM15.5 4.5v9L9.5 9z" fill="currentColor" stroke-linejoin="round" />
  </svg>
);
const ScreenshotIcon = () => (
  <svg {...iconProps}>
    <path d="M2 5.5h3l1-2h6l1 2h3v9H2z" stroke-linejoin="round" />
    <circle cx="9" cy="10" r="2.5" />
  </svg>
);
const LinkIcon = () => (
  <svg {...iconProps}>
    <path
      d="M7.5 10.5a3 3 0 0 0 4.2 0l2.8-2.8a3 3 0 0 0-4.2-4.2l-.8.8M10.5 7.5a3 3 0 0 0-4.2 0L3.5 10.3a3 3 0 0 0 4.2 4.2l.8-.8"
      stroke-linecap="round"
    />
  </svg>
);
const ResetIcon = () => (
  <svg {...iconProps}>
    <path d="M3 9a6 6 0 1 0 2-4.5M3 2.5v3h3" stroke-linejoin="round" />
  </svg>
);
const SettingsIcon = () => (
  <svg {...iconProps}>
    <path d="M7.6 3.2L7.8 1.3L10.2 1.3L10.4 3.2L12.1 3.9L13.6 2.7L15.3 4.4L14.1 5.9L14.8 7.6L16.7 7.8L16.7 10.2L14.8 10.4L14.1 12.1L15.3 13.6L13.6 15.3L12.1 14.1L10.4 14.8L10.2 16.7L7.8 16.7L7.6 14.8L5.9 14.1L4.4 15.3L2.7 13.6L3.9 12.1L3.2 10.4L1.3 10.2L1.3 7.8L3.2 7.6L3.9 5.9L2.7 4.4L4.4 2.7L5.9 3.9z" stroke-linejoin="round" />
    <circle cx="9" cy="9" r="2.3" />
  </svg>
);
const ThemeIcon = () => (
  <svg {...iconProps}>
    <Show
      when={theme() === "dark"}
      fallback={<path d="M14.5 10.5A6 6 0 0 1 7.5 3.5a6 6 0 1 0 7 7z" stroke-linejoin="round" />}
    >
      <circle cx="9" cy="9" r="3" />
      <path d="M9 1.5v2M9 14.5v2M1.5 9h2M14.5 9h2M3.7 3.7l1.4 1.4M12.9 12.9l1.4 1.4M3.7 14.3l1.4-1.4M12.9 5.1l1.4-1.4" />
    </Show>
  </svg>
);
const FullscreenIcon = (props: { on: boolean }) => (
  <svg {...iconProps}>
    <Show
      when={props.on}
      fallback={<path d="M2 6V2h4M12 2h4v4M16 12v4h-4M6 16H2v-4" />}
    >
      <path d="M6 2v4H2M16 6h-4V2M12 16v-4h4M2 12h4v4" />
    </Show>
  </svg>
);
const toggleTheme = () => setTheme(theme() === "dark" ? "light" : "dark");
const themeLabel = () => `Switch to ${theme() === "dark" ? "light" : "dark"} theme`;

const buttonClass =
  "px-2 py-1.5 rounded-md text-sm cursor-pointer text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)] inline-flex items-center gap-1.5 shrink-0";
const activeClass =
  "bg-[color:var(--color-bg-elev)] text-[color:var(--color-accent)]";
const TIMEFRAME_GROUPS = ["Minutes", "Hours", "Days"] as const;
const menuClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md shadow-lg z-50 py-1 min-w-36";
const itemClass =
  "px-3 py-1.5 text-sm cursor-pointer flex items-center gap-2 outline-none data-[highlighted]:bg-[color:var(--color-border)]";

/** True when a keystroke should be left alone (typing in a field, dialog/menu open). */
function shouldIgnoreKey(e: KeyboardEvent): boolean {
  return e.ctrlKey || e.metaKey || e.altKey || isTypingContext(e);
}

export interface ToolbarProps {
  session: McpSession;
  /** Called when the user picks the symbol or timeframe that is already active. */
  onReselect: () => void;
  /** Download the chart as a PNG. */
  onScreenshot: () => void;
  onCopyLink: () => void;
  /** Scroll to real time and restore the default bar spacing. */
  onResetChart: () => void;
  /** Open the symbol search in "add compare symbol" mode. */
  onCompare: () => void;
  /** Bar replay (ADR-0023): whether it is on, and the button's toggle. */
  replay: boolean;
  onReplay: () => void;
  /** Open the chart settings dialog. */
  onSettings: () => void;
}

export function Toolbar(props: ToolbarProps): JSX.Element {
  // Re-picking the current value refetches, so users can retry after an error.
  const setTimeframe = (tf: Timeframe) => {
    if (tf === timeframe()) props.onReselect();
    else storeSetTimeframe(tf);
  };
  const selectSymbol = (next: string) => {
    if (next.trim().toUpperCase() === symbol()) props.onReselect();
    else setSymbol(next);
  };
  const [fullscreen, setFullscreen] = createSignal(
    document.fullscreenElement != null,
  );
  const [searchOpen, setSearchOpen] = createSignal(false);
  const [searchSeed, setSearchSeed] = createSignal("");
  const [tfOpen, setTfOpen] = createSignal(false);
  const [tfSeed, setTfSeed] = createSignal("");

  const openSearch = (seed = "") => {
    setSearchSeed(seed);
    setSearchOpen(true);
  };

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (shouldIgnoreKey(e)) return;
      if (/^[a-z]$/i.test(e.key)) {
        e.preventDefault();
        openSearch(e.key.toUpperCase());
      } else if (/^[0-9]$/.test(e.key)) {
        e.preventDefault();
        setTfSeed(e.key);
        setTfOpen(true);
      }
    };
    const onFullscreen = () => setFullscreen(document.fullscreenElement != null);
    window.addEventListener("keydown", onKey);
    document.addEventListener("fullscreenchange", onFullscreen);
    onCleanup(() => {
      window.removeEventListener("keydown", onKey);
      document.removeEventListener("fullscreenchange", onFullscreen);
    });
  });

  // Lower-priority items collapse as the window narrows (see lib/viewport).
  const width = createViewportWidth();
  const showTimeframes = () => width() >= BP_TIMEFRAMES && !compact();
  const showMisc = () => width() >= BP_FULL && !compact();
  const showTools = () => width() >= BP_NARROW && !compact();
  const showReplay = () => width() >= BP_TINY && !compact();
  const hasFullscreen = document.fullscreenEnabled;
  // Timeframes in the dropdown: the favourites join it once their buttons go.
  const overflow = () =>
    showTimeframes()
      ? TIMEFRAME_OPTIONS.filter((tf) => !FAVOURITE_TIMEFRAMES.includes(tf))
      : TIMEFRAME_OPTIONS;

  return (
    <div
      class="flex items-center gap-1 shrink-0"
      role="toolbar"
      aria-label="Chart toolbar"
    >
      <button
        type="button"
        onClick={() => openSearch()}
        class={`${buttonClass} font-semibold`}
        aria-label={`Symbol: ${symbol()}. Open symbol search`}
        title="Symbol search (just start typing)"
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 14 14"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          aria-hidden="true"
        >
          <circle cx="6" cy="6" r="4" />
          <path d="M9 9l3.5 3.5" />
        </svg>
        <span class="truncate">{symbol()}</span>
      </button>

      <span class="w-px h-5 bg-[color:var(--color-border)] mx-1" />

      <div class="flex items-center" role="group" aria-label="Timeframe">
        <Show when={showTimeframes()}>
          <For each={FAVOURITE_TIMEFRAMES}>
            {(tf) => (
              <button
                type="button"
                onClick={() => setTimeframe(tf)}
                aria-pressed={timeframe() === tf}
                class={`${buttonClass} ${timeframe() === tf ? activeClass : ""}`}
              >
                {timeframeLabel(tf)}
              </button>
            )}
          </For>
        </Show>
        <DropdownMenu.Root>
          <DropdownMenu.Trigger
            class={`${buttonClass} ${
              overflow().includes(timeframe()) ? activeClass : ""
            }`}
            aria-label={
              overflow().includes(timeframe())
                ? `More timeframes (current: ${timeframeLabel(timeframe())})`
                : "More timeframes"
            }
          >
            <Show when={overflow().includes(timeframe())}>
              {timeframeLabel(timeframe())}
            </Show>
            <span aria-hidden="true">▾</span>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content class={menuClass}>
              <For each={TIMEFRAME_GROUPS}>
                {(group) => (
                  <DropdownMenu.Group>
                    <DropdownMenu.GroupLabel class="px-3 py-1 text-[10px] uppercase tracking-wide text-[color:var(--color-fg-muted)]">
                      {group}
                    </DropdownMenu.GroupLabel>
                    <For each={overflow().filter((tf) => timeframeGroup(tf) === group)}>
                      {(tf: Timeframe) => (
                        <DropdownMenu.Item
                          class={itemClass}
                          onSelect={() => setTimeframe(tf)}
                        >
                          {timeframeLabel(tf)}
                        </DropdownMenu.Item>
                      )}
                    </For>
                  </DropdownMenu.Group>
                )}
              </For>
              <DropdownMenu.Separator class="my-1 h-px bg-[color:var(--color-border)]" />
              <DropdownMenu.Item
                class={itemClass}
                onSelect={() => {
                  setTfSeed("");
                  setTfOpen(true);
                }}
              >
                Custom interval…
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      </div>

      <span class="w-px h-5 bg-[color:var(--color-border)] mx-1" />

      <DropdownMenu.Root>
        <DropdownMenu.Trigger
          class={buttonClass}
          aria-label={`Chart type: ${CHART_TYPE_LABELS[chartType()]}`}
          title="Chart type"
        >
          <ChartTypeIcon type={chartType()} />
          <span aria-hidden="true">▾</span>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content class={menuClass}>
            <DropdownMenu.RadioGroup
              value={chartType()}
              onChange={(v) => setChartType(v as ChartType)}
            >
              <For each={CHART_TYPES}>
                {(ct) => (
                  <DropdownMenu.RadioItem
                    value={ct}
                    class={itemClass}
                    closeOnSelect
                  >
                    <ChartTypeIcon type={ct} />
                    {CHART_TYPE_LABELS[ct]}
                  </DropdownMenu.RadioItem>
                )}
              </For>
            </DropdownMenu.RadioGroup>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>

      <span class="w-px h-5 bg-[color:var(--color-border)] mx-1" />

      <Show when={showTools()}>
        <button
          type="button"
          class={buttonClass}
          onClick={props.onCompare}
          aria-label="Compare symbol"
          title="Compare symbol"
        >
          <CompareIcon />
        </button>
      </Show>
      <Show when={showReplay()}>
        <button
          type="button"
          class={`${buttonClass} ${props.replay ? activeClass : ""}`}
          onClick={props.onReplay}
          aria-pressed={props.replay}
          aria-label="Bar replay"
          title="Bar replay"
          data-testid="toolbar-replay"
        >
          <ReplayIcon />
          <span class="max-[1399px]:sr-only">Replay</span>
        </button>
      </Show>
      <Show when={showMisc()}>
        <button
          type="button"
          class={buttonClass}
          onClick={props.onScreenshot}
          aria-label="Take screenshot"
          title="Download chart screenshot (PNG)"
        >
          <ScreenshotIcon />
        </button>
        <button
          type="button"
          class={buttonClass}
          onClick={props.onCopyLink}
          aria-label="Copy link to chart"
          title="Copy link to chart"
          data-testid="toolbar-copy-link"
        >
          <LinkIcon />
        </button>
      </Show>
      <Show when={showTools()}>
        <button
          type="button"
          class={buttonClass}
          onClick={props.onResetChart}
          aria-label="Reset chart"
          title="Reset chart (Alt+R)"
        >
          <ResetIcon />
        </button>
      </Show>
      <Show when={showMisc()}>
        <button
          type="button"
          class={buttonClass}
          onClick={props.onSettings}
          aria-label="Chart settings"
          title="Chart settings"
        >
          <SettingsIcon />
        </button>
      </Show>
      <Show when={showTools()}>
        <button
          type="button"
          class={buttonClass}
          onClick={toggleTheme}
          aria-label={themeLabel()}
          title="Toggle light/dark theme"
        >
          <ThemeIcon />
        </button>
      </Show>
      <Show when={showMisc() && hasFullscreen}>
        <button
          type="button"
          class={buttonClass}
          onClick={toggleFullscreen}
          aria-pressed={fullscreen()}
          aria-label="Fullscreen"
          title="Toggle fullscreen"
        >
          <FullscreenIcon on={fullscreen()} />
        </button>
      </Show>

      <Show when={!showMisc() || !showTools() || !showReplay()}>
        <DropdownMenu.Root>
          <DropdownMenu.Trigger
            class={buttonClass}
            aria-label="More tools"
            title="More tools"
            data-testid="toolbar-more"
          >
            <svg {...iconProps}>
              <g fill="currentColor" stroke="none">
                <circle cx="3.5" cy="9" r="1.5" />
                <circle cx="9" cy="9" r="1.5" />
                <circle cx="14.5" cy="9" r="1.5" />
              </g>
            </svg>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content class={menuClass}>
              <Show when={!showReplay()}>
                <DropdownMenu.Item class={itemClass} onSelect={props.onReplay}>
                  <ReplayIcon />
                  Bar replay
                </DropdownMenu.Item>
              </Show>
              <Show when={!showTools()}>
                <DropdownMenu.Item class={itemClass} onSelect={props.onCompare}>
                  <CompareIcon />
                  Compare symbol
                </DropdownMenu.Item>
                <DropdownMenu.Item class={itemClass} onSelect={props.onResetChart}>
                  <ResetIcon />
                  Reset chart
                </DropdownMenu.Item>
                <DropdownMenu.Item class={itemClass} onSelect={toggleTheme}>
                  <ThemeIcon />
                  Toggle theme
                </DropdownMenu.Item>
              </Show>
              <Show when={!showMisc()}>
                <DropdownMenu.Item class={itemClass} onSelect={props.onScreenshot}>
                  <ScreenshotIcon />
                  Take screenshot
                </DropdownMenu.Item>
                <DropdownMenu.Item class={itemClass} onSelect={props.onCopyLink}>
                  <LinkIcon />
                  Copy link to chart
                </DropdownMenu.Item>
                <DropdownMenu.Item class={itemClass} onSelect={props.onSettings}>
                  <SettingsIcon />
                  Chart settings
                </DropdownMenu.Item>
                <Show when={hasFullscreen}>
                  <DropdownMenu.Item class={itemClass} onSelect={toggleFullscreen}>
                    <FullscreenIcon on={fullscreen()} />
                    Fullscreen
                  </DropdownMenu.Item>
                </Show>
              </Show>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      </Show>

      <SymbolSearch
        open={searchOpen()}
        onOpenChange={setSearchOpen}
        initialQuery={searchSeed()}
        onSelect={selectSymbol}
        session={props.session}
      />
      <TimeframeEntry
        open={tfOpen()}
        onOpenChange={setTfOpen}
        initialValue={tfSeed()}
        onSelect={setTimeframe}
      />
    </div>
  );
}

interface TimeframeEntryProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialValue: string;
  onSelect: (tf: Timeframe) => void;
}

/** Small popup: type `15`, `2h`, `3D`... and press Enter to switch timeframe. */
function TimeframeEntry(props: TimeframeEntryProps): JSX.Element {
  let input!: HTMLInputElement;
  let previouslyFocused: HTMLElement | null = null;
  const [value, setValue] = createSignal("");
  const parsed = () => parseTimeframe(value());

  return (
    <Dialog.Root open={props.open} onOpenChange={props.onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="fixed inset-0 z-40 bg-black/40" />
        <div class="fixed inset-0 z-50 flex items-start justify-center pt-[20vh] px-4 pointer-events-none">
          <Dialog.Content
            class="pointer-events-auto w-64 bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl p-4"
            onOpenAutoFocus={(e) => {
              e.preventDefault();
              previouslyFocused =
                document.activeElement instanceof HTMLElement
                  ? document.activeElement
                  : null;
              setValue(props.initialValue);
              input.value = props.initialValue;
              input.focus();
              input.setSelectionRange(input.value.length, input.value.length);
            }}
            onCloseAutoFocus={(e) => {
              e.preventDefault();
              previouslyFocused?.focus();
            }}
          >
            <Dialog.Title class="text-sm font-semibold mb-2">
              Change timeframe
            </Dialog.Title>
            <input
              ref={input}
              type="text"
              aria-label="Timeframe, for example 15, 2h, 3D"
              autocomplete="off"
              spellcheck={false}
              value={value()}
              onInput={(e) => setValue(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key !== "Enter") return;
                e.preventDefault();
                const tf = parsed();
                if (tf === null) return;
                props.onSelect(tf);
                props.onOpenChange(false);
              }}
              class={`w-full bg-[color:var(--color-bg-elev)] border rounded-md px-3 py-2 text-sm text-[color:var(--color-fg)] focus:outline-none ${
                value() !== "" && parsed() === null
                  ? "border-[color:var(--color-danger)]"
                  : "border-[color:var(--color-border)] focus:border-[color:var(--color-accent)]"
              }`}
            />
            <p class="mt-2 text-xs text-[color:var(--color-fg-muted)]">
              <Show
                when={parsed()}
                fallback="Examples: 5, 15, 60, 2h, 3D, W"
              >
                {(tf) => `Enter to switch to ${timeframeLabel(tf())}`}
              </Show>
            </p>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

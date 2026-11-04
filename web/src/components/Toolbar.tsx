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
  timeframeLabel,
  type Timeframe,
} from "../lib/timeframe";
import {
  CHART_TYPES,
  chartType,
  setChartType,
  setSymbol,
  setTimeframe,
  symbol,
  timeframe,
  type ChartType,
} from "../stores/layout";
import { SymbolSearch } from "./SymbolSearch";

const CHART_TYPE_LABELS: Record<ChartType, string> = {
  candles: "Candles",
  hollow: "Hollow candles",
  bars: "Bars",
  line: "Line",
  area: "Area",
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
  }
}

const buttonClass =
  "px-2.5 py-1.5 rounded-md text-sm cursor-pointer text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)] inline-flex items-center gap-1.5";
const activeClass =
  "bg-[color:var(--color-bg-elev)] text-[color:var(--color-accent)]";
const menuClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md shadow-lg z-50 py-1 min-w-36";
const itemClass =
  "px-3 py-1.5 text-sm cursor-pointer flex items-center gap-2 outline-none data-[highlighted]:bg-[color:var(--color-border)]";

/** True when a keystroke should be left alone (typing in a field, dialog/menu open). */
function shouldIgnoreKey(e: KeyboardEvent): boolean {
  if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return true;
  const t = e.target;
  if (
    t instanceof HTMLElement &&
    (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))
  ) {
    return true;
  }
  return document.querySelector("[role=dialog], [role=menu]") !== null;
}

export function Toolbar(): JSX.Element {
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
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  const overflow = TIMEFRAME_OPTIONS.filter(
    (tf) => !FAVOURITE_TIMEFRAMES.includes(tf),
  );

  return (
    <div class="flex items-center gap-1" role="toolbar" aria-label="Chart toolbar">
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
        {symbol()}
      </button>

      <span class="w-px h-5 bg-[color:var(--color-border)] mx-1" />

      <div class="flex items-center" role="group" aria-label="Timeframe">
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
        <DropdownMenu.Root>
          <DropdownMenu.Trigger
            class={`${buttonClass} ${
              overflow.includes(timeframe()) ? activeClass : ""
            }`}
            aria-label="More timeframes"
          >
            <Show when={overflow.includes(timeframe())}>
              {timeframeLabel(timeframe())}
            </Show>
            <span aria-hidden="true">▾</span>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content class={menuClass}>
              <For each={overflow}>
                {(tf: Timeframe) => (
                  <DropdownMenu.Item
                    class={itemClass}
                    onSelect={() => setTimeframe(tf)}
                  >
                    {timeframeLabel(tf)}
                  </DropdownMenu.Item>
                )}
              </For>
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

      <SymbolSearch
        open={searchOpen()}
        onOpenChange={setSearchOpen}
        initialQuery={searchSeed()}
        onSelect={setSymbol}
      />
      <TimeframeEntry
        open={tfOpen()}
        onOpenChange={setTfOpen}
        initialValue={tfSeed()}
      />
    </div>
  );
}

interface TimeframeEntryProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialValue: string;
}

/** Small popup: type `15`, `4h`, `D`... and press Enter to switch timeframe. */
function TimeframeEntry(props: TimeframeEntryProps): JSX.Element {
  let input!: HTMLInputElement;
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
              setValue(props.initialValue);
              input.value = props.initialValue;
              input.focus();
              input.setSelectionRange(input.value.length, input.value.length);
            }}
          >
            <Dialog.Title class="text-sm font-semibold mb-2">
              Change timeframe
            </Dialog.Title>
            <input
              ref={input}
              type="text"
              aria-label="Timeframe, for example 15, 4h, D"
              autocomplete="off"
              spellcheck={false}
              value={value()}
              onInput={(e) => setValue(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key !== "Enter") return;
                e.preventDefault();
                const tf = parsed();
                if (tf === null) return;
                setTimeframe(tf);
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
                fallback="Examples: 5, 15, 60, 4h, D, W"
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

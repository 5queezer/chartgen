// Chart settings dialog (TradingView-style Symbol / Status line / Scales /
// Appearance tabs), ADR-0019. Edits preview live on the chart: every change
// goes through `previewChartSettings`; OK persists the draft, anything else
// that closes the dialog (Cancel, Esc, the overlay) puts the settings that
// were live when it opened back.

import { Dialog, Switch, Tabs } from "@kobalte/core";
import { For, createSignal, onCleanup, type JSX } from "solid-js";

import { resolveColors } from "../lib/chartStyle";
import { PALETTES } from "../lib/theme";
import {
  CROSSHAIR_STYLES,
  SCALE_PLACEMENTS,
  chartSettings,
  defaultSettings,
  previewChartSettings,
  saveChartSettings,
  type ChartOptions,
  type ChartSettings as Settings,
  type ColorKey,
  type PartKey,
  type ThemeOverrides,
} from "../stores/chartSettings";
import {
  invertScale,
  lockRatio,
  setInvertScale,
  setLockRatio,
  theme,
} from "../stores/layout";
import { ColorPicker } from "./ColorPicker";

export interface ChartSettingsProps {
  open: boolean;
  onClose: () => void;
}

const buttonClass =
  "border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm hover:border-[color:var(--color-accent)] cursor-pointer";
const selectClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-2 py-1 text-sm text-[color:var(--color-fg)]";
const tabClass =
  "px-3 py-2 text-sm cursor-pointer border-b-2 border-transparent text-[color:var(--color-fg-muted)] data-[selected]:text-[color:var(--color-fg)] data-[selected]:border-[color:var(--color-accent)]";
const rowClass = "flex items-center justify-between gap-4";

type BooleanOptionKey = {
  [K in keyof ChartOptions]: ChartOptions[K] extends boolean ? K : never;
}[keyof ChartOptions];

const TABS = [
  ["symbol", "Symbol"],
  ["status", "Status line"],
  ["scales", "Scales"],
  ["appearance", "Appearance"],
] as const;

function SettingsForm(props: { onClose: () => void }): JSX.Element {
  const snapshot = chartSettings();
  const [draft, setDraft] = createSignal<Settings>(snapshot);
  const [tab, setTab] = createSignal<string>("symbol");
  let committed = false;
  onCleanup(() => {
    if (!committed) previewChartSettings(snapshot);
  });

  const edit = (next: Settings) => {
    setDraft(next);
    previewChartSettings(next);
  };
  const overrides = () => draft().themes[theme()];
  const colors = () => resolveColors(PALETTES[theme()], overrides());
  const setThemePatch = (patch: ThemeOverrides) =>
    edit({
      ...draft(),
      themes: {
        ...draft().themes,
        [theme()]: { ...overrides(), ...patch },
      },
    });
  const setOptions = (patch: Partial<ChartOptions>) =>
    edit({ ...draft(), options: { ...draft().options, ...patch } });

  const submit = (e: Event) => {
    e.preventDefault();
    committed = true;
    saveChartSettings(draft());
    props.onClose();
  };

  const Toggle = (p: { label: string; option: BooleanOptionKey }) => (
    <Switch.Root
      checked={draft().options[p.option]}
      onChange={(v) => setOptions({ [p.option]: v })}
      class={rowClass}
    >
      <Switch.Label class="text-sm">{p.label}</Switch.Label>
      <Switch.Input />
      <Switch.Control class="inline-flex items-center w-9 h-5 rounded-full border border-[color:var(--color-border)] bg-[color:var(--color-bg-elev)] data-[checked]:bg-[color:var(--color-accent)] cursor-pointer transition-colors">
        <Switch.Thumb class="block w-4 h-4 rounded-full bg-white translate-x-0.5 data-[checked]:translate-x-[1.1rem] transition-transform" />
      </Switch.Control>
    </Switch.Root>
  );

  const Swatch = (p: { label: string; color: ColorKey }) => (
    <ColorPicker
      label={`${p.label} color`}
      value={colors()[p.color]}
      onChange={(v) => setThemePatch({ [p.color]: v })}
    />
  );

  const Part = (p: {
    label: string;
    part: PartKey;
    up: ColorKey;
    down: ColorKey;
  }) => (
    <div class={rowClass}>
      <label class="flex items-center gap-2 text-sm cursor-pointer">
        <input
          type="checkbox"
          aria-label={p.label}
          checked={overrides()[p.part] !== false}
          onChange={(e) => setThemePatch({ [p.part]: e.currentTarget.checked })}
        />
        {p.label}
      </label>
      <span class="flex items-center gap-2">
        <Swatch label={`${p.label} up`} color={p.up} />
        <Swatch label={`${p.label} down`} color={p.down} />
      </span>
    </div>
  );

  return (
    <form onSubmit={submit} noValidate class="flex flex-col">
      <div class="flex items-center justify-between px-4 py-3 border-b border-[color:var(--color-border)]">
        <Dialog.Title class="text-sm font-semibold">Settings</Dialog.Title>
        <Dialog.CloseButton
          aria-label="Close"
          class="text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] cursor-pointer px-2 py-1 text-sm"
        >
          ×
        </Dialog.CloseButton>
      </div>
      <Tabs.Root value={tab()} onChange={setTab}>
        <Tabs.List class="flex px-2 border-b border-[color:var(--color-border)]">
          <For each={TABS}>
            {([value, label]) => (
              <Tabs.Trigger value={value} class={tabClass}>
                {label}
              </Tabs.Trigger>
            )}
          </For>
        </Tabs.List>
        <Tabs.Content value="symbol">
          <div class="flex flex-col gap-3 p-4 max-h-[60vh] overflow-y-auto">
            <Part label="Body" part="bodyOn" up="bodyUp" down="bodyDown" />
            <Part label="Borders" part="borderOn" up="borderUp" down="borderDown" />
            <Part label="Wick" part="wickOn" up="wickUp" down="wickDown" />
            <div class={rowClass}>
              <span class="text-sm" title="Also Line with markers, Step line and High-low">
                Line and area
                <span class="block text-xs text-[color:var(--color-fg-muted)]">
                  Also markers, step line, high-low
                </span>
              </span>
              <Swatch label="Line and area" color="line" />
            </div>
          </div>
        </Tabs.Content>
        <Tabs.Content value="status">
          <div class="flex flex-col gap-3 p-4 max-h-[60vh] overflow-y-auto">
            <Toggle label="OHLC values" option="legendOhlc" />
            <Toggle label="Bar change values" option="legendChange" />
            <Toggle label="Volume" option="legendVolume" />
            <Toggle label="Indicator values" option="legendIndicators" />
          </div>
        </Tabs.Content>
        <Tabs.Content value="scales">
          <div class="flex flex-col gap-3 p-4 max-h-[60vh] overflow-y-auto">
            <Toggle label="Last price line" option="lastPriceLine" />
            <Toggle label="Last price label" option="lastPriceLabel" />
            <Toggle label="High and low price labels" option="highLowMarks" />
            {/* Per-chart scale state (like %/log), applied at once rather than by OK. */}
            <label class={`${rowClass} text-sm cursor-pointer`}>
              Invert scale
              <input
                type="checkbox"
                aria-label="Invert scale"
                checked={invertScale()}
                onChange={(e) => setInvertScale(e.currentTarget.checked)}
              />
            </label>
            <label class={`${rowClass} text-sm cursor-pointer`}>
              Lock price to bar ratio
              <input
                type="checkbox"
                aria-label="Lock price to bar ratio"
                checked={lockRatio()}
                onChange={(e) => setLockRatio(e.currentTarget.checked)}
              />
            </label>
            <div class={rowClass}>
              <label for="scale-placement" class="text-sm">
                Scale placement
              </label>
              <select
                id="scale-placement"
                class={selectClass}
                value={draft().options.scalePlacement}
                onChange={(e) =>
                  setOptions({
                    scalePlacement: e.currentTarget
                      .value as ChartOptions["scalePlacement"],
                  })
                }
              >
                <For each={SCALE_PLACEMENTS}>
                  {(p) => <option value={p}>{p}</option>}
                </For>
              </select>
            </div>
          </div>
        </Tabs.Content>
        <Tabs.Content value="appearance">
          <div class="flex flex-col gap-3 p-4 max-h-[60vh] overflow-y-auto">
            <div class={rowClass}>
              <span class="text-sm">Background</span>
              <Swatch label="Background" color="background" />
            </div>
            <Toggle label="Vertical grid lines" option="gridVertical" />
            <Toggle label="Horizontal grid lines" option="gridHorizontal" />
            <div class={rowClass}>
              <span class="text-sm">Grid color</span>
              <Swatch label="Grid" color="grid" />
            </div>
            <div class={rowClass}>
              <label for="crosshair-style" class="text-sm">
                Crosshair
              </label>
              <span class="flex items-center gap-2">
                <select
                  id="crosshair-style"
                  class={selectClass}
                  value={draft().options.crosshairStyle}
                  onChange={(e) =>
                    setOptions({
                      crosshairStyle: e.currentTarget
                        .value as ChartOptions["crosshairStyle"],
                    })
                  }
                >
                  <For each={CROSSHAIR_STYLES}>
                    {(s) => <option value={s}>{s}</option>}
                  </For>
                </select>
                <Swatch label="Crosshair" color="crosshair" />
              </span>
            </div>
          </div>
        </Tabs.Content>
      </Tabs.Root>
      <div class="flex items-center justify-between px-4 py-3 border-t border-[color:var(--color-border)]">
        <button
          type="button"
          class={buttonClass}
          title="Reset the options and the current theme's colors to the defaults"
          onClick={() =>
            edit({
              ...draft(),
              themes: { ...draft().themes, [theme()]: {} },
              options: defaultSettings().options,
            })
          }
        >
          Reset settings
        </button>
        <div class="flex gap-2">
          <button type="button" class={buttonClass} onClick={props.onClose}>
            Cancel
          </button>
          <button
            type="submit"
            class={`${buttonClass} bg-[color:var(--color-accent)] border-[color:var(--color-accent)] text-white`}
          >
            OK
          </button>
        </div>
      </div>
    </form>
  );
}

export function ChartSettings(props: ChartSettingsProps): JSX.Element {
  return (
    <Dialog.Root
      open={props.open}
      onOpenChange={(open) => {
        if (!open) props.onClose();
      }}
      modal
    >
      <Dialog.Portal>
        <Dialog.Overlay class="fixed inset-0 z-40 bg-black/60" />
        <div class="fixed inset-0 z-50 flex items-start justify-center pt-[10vh] px-4">
          <Dialog.Content class="w-full max-w-md bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl">
            <SettingsForm onClose={props.onClose} />
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

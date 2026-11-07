// Settings of the Volume Footprint and TPO chart types (ADR-0048). A change applies
// at once, to every chart; what the server computes reloads the data.

import { Dialog } from "@kobalte/core";
import { For, Show, type JSX } from "solid-js";

import {
  defaultOrderFlow,
  FOOTPRINT_DISPLAYS,
  FOOTPRINT_SUMMARIES,
  FOOTPRINT_TYPES,
  orderFlowSettings,
  saveOrderFlow,
  TPO_BLOCKS,
  TPO_PERIODS,
  TPO_STYLES,
  VP_SIDES,
  type FootprintSettings,
  type TpoSettings,
} from "../stores/orderFlow";

const LABELS: Record<string, string> = {
  buy_sell: "Buy and Sell",
  delta: "Delta",
  total: "Total",
  ladder: "Ladder",
  cluster: "Cluster",
  profile: "Profile",
  box: "Info box",
  table: "Table pane",
  none: "None",
  letters: "Letters",
  blocks: "Blocks",
  left: "Left",
  right: "Right",
};

const inputClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded px-2 py-1 text-sm w-28";

function Field(props: { label: string; children: JSX.Element }): JSX.Element {
  return (
    <label class="flex items-center justify-between gap-3 text-sm py-0.5">
      <span>{props.label}</span>
      {props.children}
    </label>
  );
}

function Check(props: { label: string; value: boolean; set: (v: boolean) => void }): JSX.Element {
  return (
    <Field label={props.label}>
      <input
        type="checkbox"
        checked={props.value}
        onChange={(e) => props.set(e.currentTarget.checked)}
      />
    </Field>
  );
}

function Num(props: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  set: (v: number) => void;
}): JSX.Element {
  return (
    <Field label={props.label}>
      <input
        type="number"
        class={inputClass}
        min={props.min}
        max={props.max}
        step={props.step ?? 1}
        value={props.value}
        onChange={(e) => {
          const v = Number(e.currentTarget.value);
          if (Number.isFinite(v) && v >= props.min && v <= props.max) props.set(v);
          else e.currentTarget.value = String(props.value);
        }}
      />
    </Field>
  );
}

function Choice<T extends string>(props: {
  label: string;
  value: T;
  options: readonly T[];
  set: (v: T) => void;
}): JSX.Element {
  return (
    <Field label={props.label}>
      <select
        class={inputClass}
        value={props.value}
        onChange={(e) => props.set(e.currentTarget.value as T)}
      >
        <For each={props.options}>
          {(o) => <option value={o} selected={o === props.value}>{LABELS[o] ?? o}</option>}
        </For>
      </select>
    </Field>
  );
}

function FootprintForm(): JSX.Element {
  const f = () => orderFlowSettings().footprint;
  const set = (patch: Partial<FootprintSettings>) =>
    saveOrderFlow({ ...orderFlowSettings(), footprint: { ...f(), ...patch } });
  return (
    <>
      <Num label="Ticks per row (0: auto)" value={f().rowTicks} min={0} max={1000000} set={(rowTicks) => set({ rowTicks })} />
      <Num label="ATR length of auto" value={f().atrLength} min={1} max={500} set={(atrLength) => set({ atrLength })} />
      <Choice label="Type" value={f().type} options={FOOTPRINT_TYPES} set={(type) => set({ type })} />
      <Choice label="Display" value={f().display} options={FOOTPRINT_DISPLAYS} set={(display) => set({ display })} />
      <Check label="Gradient background" value={f().gradient} set={(gradient) => set({ gradient })} />
      <Show when={f().gradient}>
        <Field label="Gradient colors">
          <span class="flex gap-1">
            <For each={[0, 1, 2, 3] as const}>
              {(i) => (
                <input
                  type="color"
                  aria-label={`Gradient color ${i + 1}`}
                  value={f().gradientColors[i]}
                  onChange={(e) => {
                    const gradientColors = [...f().gradientColors] as FootprintSettings["gradientColors"];
                    gradientColors[i] = e.currentTarget.value;
                    set({ gradientColors });
                  }}
                />
              )}
            </For>
          </span>
        </Field>
      </Show>
      <Check label="Value area" value={f().showValueArea} set={(showValueArea) => set({ showValueArea })} />
      <Num label="Value area %" value={f().valueArea} min={1} max={100} set={(valueArea) => set({ valueArea })} />
      <Check label="POC" value={f().showPoc} set={(showPoc) => set({ showPoc })} />
      <Choice label="Summary" value={f().summary} options={FOOTPRINT_SUMMARIES} set={(summary) => set({ summary })} />
      <Check label="Imbalances" value={f().showImbalance} set={(showImbalance) => set({ showImbalance })} />
      <Num label="Imbalance %" value={f().imbalance} min={100} max={100000} set={(imbalance) => set({ imbalance })} />
      <Check label="Stacked imbalances" value={f().showStacked} set={(showStacked) => set({ showStacked })} />
      <Num label="Levels in a stack" value={f().stacked} min={1} max={100} set={(stacked) => set({ stacked })} />
    </>
  );
}

function TpoForm(): JSX.Element {
  const t = () => orderFlowSettings().tpo;
  const set = (patch: Partial<TpoSettings>) =>
    saveOrderFlow({ ...orderFlowSettings(), tpo: { ...t(), ...patch } });
  return (
    <>
      <Choice label="Period" value={t().period} options={TPO_PERIODS} set={(period) => set({ period })} />
      <Choice label="Block size" value={t().block} options={TPO_BLOCKS} set={(block) => set({ block })} />
      <Num label="Ticks per row (0: auto)" value={t().rowTicks} min={0} max={1000000} set={(rowTicks) => set({ rowTicks })} />
      <Num label="Value area %" value={t().valueArea} min={1} max={100} set={(valueArea) => set({ valueArea })} />
      <Choice label="Draw as" value={t().style} options={TPO_STYLES} set={(style) => set({ style })} />
      <Check label="Split by blocks" value={t().splitByBlocks} set={(splitByBlocks) => set({ splitByBlocks })} />
      <Num label="Opacity outside value area" value={t().outsideOpacity} min={0} max={1} step={0.05} set={(outsideOpacity) => set({ outsideOpacity })} />
      <Check label="POC" value={t().showPoc} set={(showPoc) => set({ showPoc })} />
      <Check label="VAH and VAL" value={t().showValueArea} set={(showValueArea) => set({ showValueArea })} />
      <Check label="Poor high and low" value={t().showPoor} set={(showPoor) => set({ showPoor })} />
      <Check label="Single prints" value={t().showSingles} set={(showSingles) => set({ showSingles })} />
      <Check label="Midpoint" value={t().showMidpoint} set={(showMidpoint) => set({ showMidpoint })} />
      <Check label="Open and close" value={t().showOpenClose} set={(showOpenClose) => set({ showOpenClose })} />
      <Check label="Initial balance" value={t().showIb} set={(showIb) => set({ showIb })} />
      <Num label="Initial balance blocks" value={t().ibBlocks} min={1} max={100} set={(ibBlocks) => set({ ibBlocks })} />
      <Check label="Volume profile" value={t().showProfile} set={(showProfile) => set({ showProfile })} />
      <Choice label="Profile side" value={t().profileSide} options={VP_SIDES} set={(profileSide) => set({ profileSide })} />
      <Check label="Summary pane" value={t().summary} set={(summary) => set({ summary })} />
      <Field label="Sessions (HH:MM-HH:MM, commas)">
        <input
          type="text"
          class={inputClass}
          value={t().sessions}
          placeholder="All"
          onChange={(e) => set({ sessions: e.currentTarget.value })}
        />
      </Field>
    </>
  );
}

export function OrderFlowSettingsDialog(props: {
  kind: "footprint" | "tpo" | null;
  open: boolean;
  onClose: () => void;
}): JSX.Element {
  const reset = () => {
    const d = defaultOrderFlow();
    const kind = props.kind;
    if (kind) saveOrderFlow({ ...orderFlowSettings(), [kind]: d[kind] });
  };
  return (
    <Dialog.Root open={props.open} onOpenChange={(open) => !open && props.onClose()} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="fixed inset-0 z-40 bg-black/60" />
        <div class="fixed inset-0 z-50 flex items-start justify-center pt-[6vh] px-4">
          <Dialog.Content
            data-testid="orderflow-settings"
            class="w-full max-w-sm max-h-[85vh] overflow-y-auto bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl p-4"
          >
            <Dialog.Title class="font-semibold mb-2">
              {props.kind === "tpo" ? "TPO settings" : "Footprint settings"}
            </Dialog.Title>
            <Show when={props.kind === "tpo"} fallback={<FootprintForm />}>
              <TpoForm />
            </Show>
            <div class="flex justify-end gap-2 mt-3">
              <button type="button" class="px-3 py-1.5 rounded-md text-sm border border-[color:var(--color-border)] cursor-pointer" onClick={reset}>
                Reset
              </button>
              <button type="button" class="px-3 py-1.5 rounded-md text-sm bg-[color:var(--color-accent)] text-black font-semibold cursor-pointer" onClick={props.onClose}>
                Close
              </button>
            </div>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

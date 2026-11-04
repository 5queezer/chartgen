// Indicator settings dialog (TradingView-style "Inputs" and "Style" tabs).
// The Inputs fields are generated from the indicator's `configurable_params`
// schema by lib/paramForm; the Style tab edits per-plot color, width, line
// style and visibility (ADR-0017). OK validates and hands both back to the
// caller; Defaults restores the defaults of the tab being shown in the form
// (not yet applied).

import { Dialog, Switch, Tabs } from "@kobalte/core";
import { For, Show, createMemo, createSignal, type JSX } from "solid-js";

import { resolvePlot, type PlotInfo } from "../lib/plotStyle";
import {
  sanitizeStyle,
  type IndicatorStyle,
  type PlotLineStyle,
  type PlotStyle,
} from "../stores/indicators";
import { ColorPicker } from "./ColorPicker";

import {
  buildFields,
  defaultValues,
  initialValues,
  validate,
  type FormValues,
  type ParamField,
} from "../lib/paramForm";
import type { ParamSpec } from "../types/responses";

export interface IndicatorSettingsProps {
  /** Indicator being edited, or null when the dialog is closed. */
  target: {
    name: string;
    params: Record<string, unknown>;
    style: IndicatorStyle | undefined;
  } | null;
  /** Plots of the indicator in the current series payload, with their default colors. */
  plots: readonly PlotInfo[];
  /** `configurable_params` for the target; undefined while the registry is unavailable. */
  specs: readonly ParamSpec[] | undefined;
  onClose: () => void;
  /**
   * Called with the validated params, the fields they were validated
   * against and the plot style (undefined when no override remains).
   */
  onSubmit: (
    params: Record<string, unknown>,
    fields: ParamField[],
    style: IndicatorStyle | undefined,
  ) => void;
}

const inputClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)] w-32 text-right tabular-nums";
const buttonClass =
  "border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm hover:border-[color:var(--color-accent)] cursor-pointer";

const selectClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-2 py-1 text-sm text-[color:var(--color-fg)] disabled:opacity-40";
const tabClass =
  "px-3 py-2 text-sm cursor-pointer border-b-2 border-transparent text-[color:var(--color-fg-muted)] data-[selected]:text-[color:var(--color-fg)] data-[selected]:border-[color:var(--color-accent)]";

const LINE_STYLES: readonly PlotLineStyle[] = ["solid", "dashed", "dotted"];

function StyleRows(props: {
  plots: readonly PlotInfo[];
  styles: Record<string, PlotStyle>;
  onChange: (key: string, patch: PlotStyle) => void;
}): JSX.Element {
  return (
    <Show
      when={props.plots.length > 0}
      fallback={
        <p class="text-sm text-[color:var(--color-fg-muted)]">
          This indicator has no lines to style.
        </p>
      }
    >
      <For each={props.plots}>
        {(plot) => {
          const r = () => resolvePlot(plot, { plots: props.styles });
          const isLine = plot.kind === "line";
          return (
            <div class="flex items-center gap-3" data-testid={`style-row-${plot.key}`}>
              <input
                type="checkbox"
                aria-label={`Show ${plot.label}`}
                checked={r().visible}
                onChange={(e) =>
                  props.onChange(plot.key, { visible: e.currentTarget.checked })
                }
                class="cursor-pointer"
              />
              <span class="flex-1 min-w-0 truncate text-sm font-mono">{plot.label}</span>
              <ColorPicker
                label={`${plot.label} color`}
                value={r().color}
                onChange={(color) => props.onChange(plot.key, { color })}
              />
              <select
                aria-label={`${plot.label} width`}
                class={selectClass}
                disabled={!isLine}
                value={r().width}
                onChange={(e) =>
                  props.onChange(plot.key, { width: Number(e.currentTarget.value) })
                }
              >
                <For each={[1, 2, 3, 4]}>{(w) => <option value={w}>{w}px</option>}</For>
              </select>
              <select
                aria-label={`${plot.label} line style`}
                class={selectClass}
                disabled={!isLine}
                value={r().lineStyle}
                onChange={(e) =>
                  props.onChange(plot.key, {
                    lineStyle: e.currentTarget.value as PlotLineStyle,
                  })
                }
              >
                <For each={LINE_STYLES}>{(l) => <option value={l}>{l}</option>}</For>
              </select>
            </div>
          );
        }}
      </For>
    </Show>
  );
}

function SettingsForm(props: {
  name: string;
  params: Record<string, unknown>;
  style: IndicatorStyle | undefined;
  plots: readonly PlotInfo[];
  specs: readonly ParamSpec[];
  onClose: () => void;
  onSubmit: IndicatorSettingsProps["onSubmit"];
}): JSX.Element {
  const fields = createMemo(() => buildFields(props.specs));
  const [values, setValues] = createSignal<FormValues>(
    initialValues(fields(), props.params),
  );
  const [errors, setErrors] = createSignal<Record<string, string>>({});
  const [tab, setTab] = createSignal("inputs");
  const [plotStyles, setPlotStyles] = createSignal<Record<string, PlotStyle>>(
    props.style?.plots ?? {},
  );

  const setValue = (name: string, value: string | boolean) =>
    setValues({ ...values(), [name]: value });

  const setPlot = (key: string, patch: PlotStyle) =>
    setPlotStyles({ ...plotStyles(), [key]: { ...plotStyles()[key], ...patch } });

  const submit = (e: Event) => {
    e.preventDefault();
    const result = validate(fields(), values());
    setErrors(result.errors);
    if (Object.keys(result.errors).length > 0) {
      setTab("inputs");
      return;
    }
    props.onSubmit(result.params, fields(), sanitizeStyle({ plots: plotStyles() }));
  };

  return (
    <form onSubmit={submit} noValidate class="flex flex-col">
      <div class="flex items-center justify-between px-4 py-3 border-b border-[color:var(--color-border)]">
        <Dialog.Title class="text-sm font-semibold">{props.name}</Dialog.Title>
        <Dialog.CloseButton
          aria-label="Close"
          class="text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] cursor-pointer px-2 py-1 text-sm"
        >
          ×
        </Dialog.CloseButton>
      </div>
      <Tabs.Root value={tab()} onChange={setTab}>
        <Tabs.List class="flex px-2 border-b border-[color:var(--color-border)]">
          <Tabs.Trigger value="inputs" class={tabClass}>
            Inputs
          </Tabs.Trigger>
          <Tabs.Trigger value="style" class={tabClass}>
            Style
          </Tabs.Trigger>
        </Tabs.List>
        <Tabs.Content value="inputs">
      <div class="flex flex-col gap-3 p-4 max-h-[60vh] overflow-y-auto">
        <Show
          when={fields().length > 0}
          fallback={
            <p class="text-sm text-[color:var(--color-fg-muted)]">
              This indicator has no settings.
            </p>
          }
        >
          <For each={fields()}>
            {(f) => (
              <div class="flex flex-col gap-1">
                <div class="flex items-center justify-between gap-4">
                  <label
                    for={`param-${f.name}`}
                    class="text-sm font-mono text-[color:var(--color-fg)]"
                  >
                    {f.name}
                  </label>
                  <Show
                    when={f.kind === "boolean"}
                    fallback={
                      <input
                        id={`param-${f.name}`}
                        type={f.kind === "string" ? "text" : "number"}
                        step={f.step}
                        min={f.min}
                        max={f.max}
                        value={values()[f.name] as string}
                        onInput={(e) => setValue(f.name, e.currentTarget.value)}
                        aria-invalid={errors()[f.name] ? "true" : undefined}
                        class={inputClass}
                      />
                    }
                  >
                    <Switch.Root
                      checked={values()[f.name] === true}
                      onChange={(v) => setValue(f.name, v)}
                      class="inline-flex"
                    >
                      <Switch.Input id={`param-${f.name}`} />
                      <Switch.Control class="inline-flex items-center w-9 h-5 rounded-full border border-[color:var(--color-border)] bg-[color:var(--color-bg-elev)] data-[checked]:bg-[color:var(--color-accent)] cursor-pointer transition-colors">
                        <Switch.Thumb class="block w-4 h-4 rounded-full bg-white translate-x-0.5 data-[checked]:translate-x-[1.1rem] transition-transform" />
                      </Switch.Control>
                    </Switch.Root>
                  </Show>
                </div>
                <Show when={f.description}>
                  <span class="text-xs text-[color:var(--color-fg-muted)]">
                    {f.description}
                  </span>
                </Show>
                <Show when={errors()[f.name]}>
                  <span role="alert" class="text-xs text-[color:var(--color-danger)]">
                    {errors()[f.name]}
                  </span>
                </Show>
              </div>
            )}
          </For>
        </Show>
      </div>
        </Tabs.Content>
        <Tabs.Content value="style">
          <div class="flex flex-col gap-3 p-4 max-h-[60vh] overflow-y-auto">
            <StyleRows plots={props.plots} styles={plotStyles()} onChange={setPlot} />
          </div>
        </Tabs.Content>
      </Tabs.Root>
      <div class="flex items-center justify-between px-4 py-3 border-t border-[color:var(--color-border)]">
        <button
          type="button"
          class={buttonClass}
          disabled={tab() === "inputs" ? fields().length === 0 : props.plots.length === 0}
          onClick={() => {
            if (tab() === "inputs") {
              setValues(defaultValues(fields()));
              setErrors({});
            } else {
              setPlotStyles({});
            }
          }}
        >
          Defaults
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

export function IndicatorSettings(props: IndicatorSettingsProps): JSX.Element {
  return (
    <Dialog.Root
      open={props.target !== null}
      onOpenChange={(open) => {
        if (!open) props.onClose();
      }}
      modal
    >
      <Dialog.Portal>
        <Dialog.Overlay class="fixed inset-0 z-40 bg-black/60" />
        <div class="fixed inset-0 z-50 flex items-start justify-center pt-[10vh] px-4">
          <Dialog.Content class="w-full max-w-md bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl">
            <Show when={props.target} keyed>
              {(t) => (
                // The form seeds its values from the schema once, so it is
                // only built when the schema is available (never an empty
                // form whose OK would wipe the params).
                <Show
                  when={props.specs}
                  fallback={
                    <div class="flex flex-col gap-3 p-4">
                      <Dialog.Title class="text-sm font-semibold">
                        {t.name}
                      </Dialog.Title>
                      <p class="text-sm text-[color:var(--color-fg-muted)]">
                        Settings unavailable: the indicator list is loading or
                        failed to load.
                      </p>
                      <div class="flex justify-end gap-2">
                        <button type="button" class={buttonClass} onClick={props.onClose}>
                          Cancel
                        </button>
                        <button type="button" class={buttonClass} disabled>
                          OK
                        </button>
                      </div>
                    </div>
                  }
                >
                  {(specs) => (
                    <SettingsForm
                      name={t.name}
                      params={t.params}
                      style={t.style}
                      plots={props.plots}
                      specs={specs()}
                      onClose={props.onClose}
                      onSubmit={props.onSubmit}
                    />
                  )}
                </Show>
              )}
            </Show>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

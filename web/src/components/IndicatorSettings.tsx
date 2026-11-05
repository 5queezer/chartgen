// Indicator settings dialog (TradingView-style "Inputs" and "Style" tabs).
// The Inputs fields are generated from the indicator's `configurable_params`
// schema by lib/paramForm; the Style tab edits per-plot color, width, line
// style and visibility (ADR-0017) and the colors of its fills, levels,
// volume-profile bars, divergence lines and histogram palettes. OK validates and hands both back to the
// caller; Defaults restores the defaults of the tab being shown in the form
// (not yet applied).

import { Dialog, Switch, Tabs } from "@kobalte/core";
import { For, Index, Show, createMemo, createSignal, type JSX } from "solid-js";

import { pruneGeometry, type GeometryInfo } from "../lib/geometryStyle";
import { rekeyPlots, resolvePlot, type PlotInfo } from "../lib/plotStyle";
import {
  sanitizeStyle,
  type GeoItemStyle,
  type GeometryStyle,
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
    /** Dialog title when it differs from `name`, e.g. `RSI (21)` for a repeated indicator. */
    title?: string;
    params: Record<string, unknown>;
    style: IndicatorStyle | undefined;
    hidden: boolean;
  } | null;
  /**
   * Plots of the indicator in the current series payload, with their default
   * colors; null when the payload has no entry for it (e.g. it is hidden).
   */
  plots: readonly PlotInfo[] | null;
  /** Fills, levels, profile bars and divergence lines of the indicator; null like `plots`. */
  geometry: GeometryInfo | null;
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

// More palette colors than this: one swatch for the whole histogram.
const MAX_PALETTE_SWATCHES = 16;
const LINE_STYLES: readonly PlotLineStyle[] = ["solid", "dashed", "dotted"];

interface Swatch {
  label: string;
  value: string;
  onChange: (color: string) => void;
}

/** One Style tab row: visibility checkbox, label, color swatches and extra controls. */
function GeoRow(props: {
  testId: string;
  label: string;
  visible: boolean;
  onVisible: (visible: boolean) => void;
  swatches: Swatch[];
  children?: JSX.Element;
}): JSX.Element {
  return (
    <div class="flex items-center gap-3" data-testid={props.testId}>
      <input
        type="checkbox"
        aria-label={`Show ${props.label}`}
        checked={props.visible}
        onChange={(e) => props.onVisible(e.currentTarget.checked)}
        class="cursor-pointer"
      />
      <span class="flex-1 min-w-0 truncate text-sm font-mono">{props.label}</span>
      {/* Index, not For: the swatches are rebuilt on every change, and a
          remounted ColorPicker would close its popover. */}
      <Index each={props.swatches}>
        {(sw) => (
          <ColorPicker
            label={sw().label}
            value={sw().value}
            onChange={sw().onChange}
          />
        )}
      </Index>
      {props.children}
    </div>
  );
}

function StyleRows(props: {
  plots: readonly PlotInfo[] | null;
  geometry: GeometryInfo | null;
  hidden: boolean;
  styles: Record<string, PlotStyle>;
  geo: GeometryStyle;
  onChange: (plot: PlotInfo, patch: PlotStyle) => void;
  onGeo: (patch: (g: GeometryStyle) => GeometryStyle) => void;
}): JSX.Element {
  const item = (group: "fills" | "hlines", i: number, patch: GeoItemStyle) =>
    props.onGeo((g) => ({
      ...g,
      [group]: { ...g[group], [i]: { ...g[group]?.[i], ...patch } },
    }));
  return (
    <Show
      when={!props.hidden && props.plots}
      fallback={
        <p class="text-sm text-[color:var(--color-fg-muted)]">
          Show the indicator to edit its style.
        </p>
      }
    >
      {(plots) => (
        <Show
          when={
            plots().length > 0 ||
            (props.geometry &&
              (props.geometry.fills.length > 0 ||
                props.geometry.hlines.length > 0 ||
                props.geometry.hbars ||
                props.geometry.divlines))
          }
          fallback={
            <p class="text-sm text-[color:var(--color-fg-muted)]">
              This indicator has no lines to style.
            </p>
          }
        >
          <p class="text-xs text-[color:var(--color-fg-muted)]">
            Changes to the lines on the Inputs tab apply after OK.
          </p>
          {/* Index throughout: the plots and geometry are rebuilt on every
              payload (live ticks), which must not remount an open popover. */}
          <Index each={plots()}>
            {(plotAcc) => {
              const plot = () => plotAcc();
              const r = () => resolvePlot(plot(), { plots: props.styles });
              const isLine = () => plot().kind === "line";
              const own = () => props.styles[plot().key];
              const swatches = (): Swatch[] => {
                const defaults = plot().palette;
                if (
                  isLine() ||
                  !defaults ||
                  defaults.length < 2 ||
                  defaults.length > MAX_PALETTE_SWATCHES
                ) {
                  return [
                    {
                      label: `${plot().label} color`,
                      value: r().color,
                      onChange: (color) => props.onChange(plot(), { color }),
                    },
                  ];
                }
                // A legacy single color recolors every bar and wins over the
                // palette; editing a palette color replaces it.
                return defaults.map((d, k) => ({
                  label: `${plot().label} color ${k + 1}`,
                  value: own()?.color ?? own()?.palette?.[d] ?? d,
                  onChange: (color) =>
                    props.onChange(plot(), {
                      palette: { ...own()?.palette, [d]: color },
                      color: undefined,
                    }),
                }));
              };
              return (
                <GeoRow
                  testId={`style-row-${plot().key}`}
                  label={plot().label}
                  visible={r().visible}
                  onVisible={(visible) => props.onChange(plot(), { visible })}
                  swatches={swatches()}
                >
                  <select
                    aria-label={`${plot().label} width`}
                    class={selectClass}
                    disabled={!isLine()}
                    value={r().width}
                    onChange={(e) =>
                      props.onChange(plot(), {
                        width: Number(e.currentTarget.value),
                      })
                    }
                  >
                    <For each={[1, 2, 3, 4]}>
                      {(w) => <option value={w}>{w}px</option>}
                    </For>
                  </select>
                  <select
                    aria-label={`${plot().label} line style`}
                    class={selectClass}
                    disabled={!isLine()}
                    value={r().lineStyle}
                    onChange={(e) =>
                      props.onChange(plot(), {
                        lineStyle: e.currentTarget.value as PlotLineStyle,
                      })
                    }
                  >
                    <For each={LINE_STYLES}>
                      {(l) => <option value={l}>{l}</option>}
                    </For>
                  </select>
                </GeoRow>
              );
            }}
          </Index>
          <Show when={props.geometry}>
            {(geometry) => (
              <>
                <Index each={geometry().fills}>
                  {(f, i) => {
                    const o = () => props.geo.fills?.[i];
                    const guard = () => ({ ref: f().ref });
                    return (
                      <GeoRow
                        testId={`style-fill-${i}`}
                        label={f().label}
                        visible={o()?.visible !== false}
                        onVisible={(visible) => item("fills", i, { visible, ...guard() })}
                        swatches={[
                          {
                            label: `${f().label} color`,
                            value: o()?.color ?? f().up,
                            onChange: (color) => item("fills", i, { color, ...guard() }),
                          },
                          ...(f().twoTone
                            ? [
                                {
                                  label: `${f().label} color below`,
                                  value: o()?.colorBelow ?? f().down,
                                  onChange: (colorBelow: string) =>
                                    item("fills", i, { colorBelow, ...guard() }),
                                },
                              ]
                            : []),
                        ]}
                      />
                    );
                  }}
                </Index>
                <Index each={geometry().hlines}>
                  {(h, i) => {
                    const o = () => props.geo.hlines?.[i];
                    // Profile levels move with every payload: no `y` guard.
                    const guard = () => ({
                      ref: h().ref,
                      ...(geometry().hbars ? {} : { y: h().y }),
                    });
                    return (
                      <GeoRow
                        testId={`style-hline-${i}`}
                        label={`Level ${h().label}`}
                        visible={o()?.visible !== false}
                        onVisible={(visible) => item("hlines", i, { visible, ...guard() })}
                        swatches={[
                          {
                            label: `Level ${h().label} color`,
                            value: o()?.color ?? h().color,
                            onChange: (color) => item("hlines", i, { color, ...guard() }),
                          },
                        ]}
                      >
                        <select
                          aria-label={`Level ${h().label} line style`}
                          class={selectClass}
                          value={o()?.lineStyle ?? "solid"}
                          onChange={(e) =>
                            item("hlines", i, {
                              lineStyle: e.currentTarget.value as PlotLineStyle,
                              ...guard(),
                            })
                          }
                        >
                          <For each={LINE_STYLES}>
                            {(l) => <option value={l}>{l}</option>}
                          </For>
                        </select>
                      </GeoRow>
                    );
                  }}
                </Index>
                <Show when={geometry().hbars}>
                  {(b) => {
                    const setBars = (patch: NonNullable<GeometryStyle["hbars"]>) =>
                      props.onGeo((g) => ({ ...g, hbars: { ...g.hbars, ...patch } }));
                    return (
                      <GeoRow
                        testId="style-hbars"
                        label="Profile bars"
                        visible={props.geo.hbars?.visible !== false}
                        onVisible={(visible) => setBars({ visible })}
                        swatches={[
                          {
                            label: "Profile bars color",
                            value: props.geo.hbars?.color ?? b().color,
                            onChange: (color) => setBars({ color }),
                          },
                          ...(b().va !== null
                            ? [
                                {
                                  label: "Profile value area color",
                                  value: props.geo.hbars?.vaColor ?? b().va!,
                                  onChange: (vaColor: string) => setBars({ vaColor }),
                                },
                              ]
                            : []),
                          {
                            label: "Profile POC color",
                            value: props.geo.hbars?.pocColor ?? b().pocColor,
                            onChange: (pocColor) => setBars({ pocColor }),
                          },
                        ]}
                      />
                    );
                  }}
                </Show>
                <Show when={geometry().divlines}>
                  {(d) => {
                    const setDiv = (patch: NonNullable<GeometryStyle["divlines"]>) =>
                      props.onGeo((g) => ({ ...g, divlines: { ...g.divlines, ...patch } }));
                    return (
                      <GeoRow
                        testId="style-divlines"
                        label="Divergence lines"
                        visible={props.geo.divlines?.visible !== false}
                        onVisible={(visible) => setDiv({ visible })}
                        swatches={[
                          {
                            label: "Bullish divergence color",
                            value: props.geo.divlines?.bullColor ?? d().bull,
                            onChange: (bullColor) => setDiv({ bullColor }),
                          },
                          {
                            label: "Bearish divergence color",
                            value: props.geo.divlines?.bearColor ?? d().bear,
                            onChange: (bearColor) => setDiv({ bearColor }),
                          },
                        ]}
                      />
                    );
                  }}
                </Show>
              </>
            )}
          </Show>
        </Show>
      )}
    </Show>
  );
}

function SettingsForm(props: {
  name: string;
  params: Record<string, unknown>;
  style: IndicatorStyle | undefined;
  hidden: boolean;
  plots: readonly PlotInfo[] | null;
  geometry: GeometryInfo | null;
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
    props.plots
      ? rekeyPlots(props.plots, props.style)
      : (props.style?.plots ?? {}),
  );

  // Overrides for fills, levels or groups the payload no longer has are dropped.
  const [geoStyle, setGeoStyle] = createSignal<GeometryStyle>(
    props.style?.geometry && props.geometry
      ? pruneGeometry(props.style.geometry, props.geometry)
      : (props.style?.geometry ?? {}),
  );

  const setValue = (name: string, value: string | boolean) =>
    setValues({ ...values(), [name]: value });

  // Index-keyed lines record their label so the entry is ignored if another
  // line later takes that index.
  const setPlot = (plot: PlotInfo, patch: PlotStyle) =>
    setPlotStyles({
      ...plotStyles(),
      [plot.key]: {
        ...plotStyles()[plot.key],
        ...patch,
        ...(plot.kind === "line" && plot.indexKeyed
          ? { label: plot.label }
          : {}),
      },
    });

  const submit = (e: Event) => {
    e.preventDefault();
    const result = validate(fields(), values());
    setErrors(result.errors);
    if (Object.keys(result.errors).length > 0) {
      setTab("inputs");
      return;
    }
    props.onSubmit(
      result.params,
      fields(),
      sanitizeStyle({ plots: plotStyles(), geometry: geoStyle() }),
    );
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
                            onInput={(e) =>
                              setValue(f.name, e.currentTarget.value)
                            }
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
                      <span
                        role="alert"
                        class="text-xs text-[color:var(--color-danger)]"
                      >
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
            <StyleRows
              plots={props.plots}
              geometry={props.geometry}
              hidden={props.hidden}
              styles={plotStyles()}
              geo={geoStyle()}
              onChange={setPlot}
              onGeo={(patch) => setGeoStyle(patch(geoStyle()))}
            />
          </div>
        </Tabs.Content>
      </Tabs.Root>
      <div class="flex items-center justify-between px-4 py-3 border-t border-[color:var(--color-border)]">
        <button
          type="button"
          class={buttonClass}
          disabled={
            tab() === "inputs"
              ? fields().length === 0
              : Object.keys(plotStyles()).length === 0 &&
                Object.keys(geoStyle()).length === 0
          }
          onClick={() => {
            if (tab() === "inputs") {
              setValues(defaultValues(fields()));
              setErrors({});
            } else {
              setPlotStyles({});
              setGeoStyle({});
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
                        {t.title ?? t.name}
                      </Dialog.Title>
                      <p class="text-sm text-[color:var(--color-fg-muted)]">
                        Settings unavailable: the indicator list is loading or
                        failed to load.
                      </p>
                      <div class="flex justify-end gap-2">
                        <button
                          type="button"
                          class={buttonClass}
                          onClick={props.onClose}
                        >
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
                      name={t.title ?? t.name}
                      params={t.params}
                      style={t.style}
                      hidden={t.hidden}
                      plots={props.plots}
                      geometry={props.geometry}
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

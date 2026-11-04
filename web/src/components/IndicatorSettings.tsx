// Indicator settings dialog (TradingView-style "Inputs" form). The fields
// are generated from the indicator's `configurable_params` schema by
// lib/paramForm; OK validates and hands the parsed params back to the
// caller, Defaults restores the schema defaults in the form (not yet applied).

import { Dialog, Switch } from "@kobalte/core";
import { For, Show, createMemo, createSignal, type JSX } from "solid-js";

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
  target: { name: string; params: Record<string, unknown> } | null;
  /** `configurable_params` for the target; undefined while the registry is unavailable. */
  specs: readonly ParamSpec[] | undefined;
  onClose: () => void;
  /** Called with the validated params and the fields they were validated against. */
  onSubmit: (params: Record<string, unknown>, fields: ParamField[]) => void;
}

const inputClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)] w-32 text-right tabular-nums";
const buttonClass =
  "border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm hover:border-[color:var(--color-accent)] cursor-pointer";

function SettingsForm(props: {
  name: string;
  params: Record<string, unknown>;
  specs: readonly ParamSpec[];
  onClose: () => void;
  onSubmit: IndicatorSettingsProps["onSubmit"];
}): JSX.Element {
  const fields = createMemo(() => buildFields(props.specs));
  const [values, setValues] = createSignal<FormValues>(
    initialValues(fields(), props.params),
  );
  const [errors, setErrors] = createSignal<Record<string, string>>({});

  const setValue = (name: string, value: string | boolean) =>
    setValues({ ...values(), [name]: value });

  const submit = (e: Event) => {
    e.preventDefault();
    const result = validate(fields(), values());
    setErrors(result.errors);
    if (Object.keys(result.errors).length === 0) props.onSubmit(result.params, fields());
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
      <div class="flex items-center justify-between px-4 py-3 border-t border-[color:var(--color-border)]">
        <button
          type="button"
          class={buttonClass}
          disabled={fields().length === 0}
          onClick={() => {
            setValues(defaultValues(fields()));
            setErrors({});
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
          <Dialog.Content class="w-full max-w-sm bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl">
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

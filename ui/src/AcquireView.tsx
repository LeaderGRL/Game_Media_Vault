import { FormEvent, useRef, useState } from "react";

import {
  ASSET_TYPE_FAMILIES,
  AcquisitionForm,
  AcquisitionPlan,
  AcquisitionRequestDraft,
  KNOWN_SOURCES,
  RetentionPolicy,
  assetTypeLabel,
  buildAcquisitionRequest,
  emptyAcquisitionForm,
  pixelSizeProblem,
  sourceLabel,
} from "./acquisition";
import {
  BUILT_IN_PRESETS,
  RequestPreset,
  applyPreset,
  defaultPresetStorage,
  deleteCustomPreset,
  isBuiltInPresetName,
  loadCustomPresets,
  renameCustomPreset,
  saveCustomPreset,
} from "./presets";
import { errorMessage } from "./types";

interface AcquireViewProps {
  starting: boolean;
  onStart: (request: AcquisitionRequestDraft) => void;
  /** Explains which Sources the request would contact; the button shows only with it. */
  onCheckPlan?: (request: AcquisitionRequestDraft) => Promise<AcquisitionPlan>;
  /** Where custom presets persist; the webview's local storage by default. */
  presetStorage?: Storage;
}

/**
 * Compact Acquisition Request builder; the backend validates the submitted draft, except pixel
 * sizes the request could not carry, which are refused here.
 */
export function AcquireView({
  starting,
  onStart,
  onCheckPlan,
  presetStorage: givenPresetStorage,
}: AcquireViewProps) {
  // Kept for the life of the view, so a session-only fallback keeps what it saved.
  const [presetStorage] = useState(() => givenPresetStorage ?? defaultPresetStorage());
  const [customPresets, setCustomPresets] = useState<RequestPreset[]>(() =>
    loadCustomPresets(presetStorage),
  );
  const [presetName, setPresetName] = useState(BUILT_IN_PRESETS[0].name);
  const [presetDraftName, setPresetDraftName] = useState("");
  const [presetProblem, setPresetProblem] = useState<string | null>(null);
  const selectedPreset = [...BUILT_IN_PRESETS, ...customPresets].find(
    (preset) => preset.name === presetName,
  );
  const customSelected = customPresets.some((preset) => preset.name === presetName);
  const [form, setForm] = useState<AcquisitionForm>(emptyAcquisitionForm);
  const [formProblem, setFormProblem] = useState<string | null>(null);
  const [plan, setPlan] = useState<AcquisitionPlan | null>(null);
  const [planProblem, setPlanProblem] = useState<string | null>(null);
  // The latest plan check; an earlier one or one made before the request changed is dropped.
  const planCheckRef = useRef<object | null>(null);

  function update(change: Partial<AcquisitionForm>) {
    setForm((current) => ({ ...current, ...change }));
    // A plan explains the request it was checked for only.
    planCheckRef.current = null;
    setPlan(null);
    setPlanProblem(null);
  }

  async function checkPlan() {
    const problem = pixelSizeProblem(form);
    setFormProblem(problem);
    if (problem !== null || onCheckPlan === undefined) {
      return;
    }
    const check = {};
    planCheckRef.current = check;
    setPlan(null);
    setPlanProblem(null);
    try {
      const checked = await onCheckPlan(buildAcquisitionRequest(form));
      if (planCheckRef.current === check) {
        setPlan(checked);
      }
    } catch (reason) {
      if (planCheckRef.current === check) {
        setPlanProblem(errorMessage(reason));
      }
    }
  }

  /** Runs `change` on the saved presets, then reads them back as a new session would. */
  function changePresets(change: () => void): boolean {
    try {
      change();
      setPresetProblem(null);
      return true;
    } catch {
      setPresetProblem("Presets cannot be saved in this window.");
      return false;
    } finally {
      setCustomPresets(loadCustomPresets(presetStorage));
    }
  }

  /** The typed preset name, or `null` after saying why it cannot name a saved preset. */
  function draftName(): string | null {
    const name = presetDraftName.trim();
    if (name.length === 0) {
      setPresetProblem("Name the preset first.");
      return null;
    }
    if (isBuiltInPresetName(name)) {
      setPresetProblem(`"${name}" is a built-in preset.`);
      return null;
    }
    return name;
  }

  function savePreset() {
    const name = draftName();
    if (name !== null && changePresets(() => saveCustomPreset(presetStorage, name, form))) {
      setPresetName(name);
    }
  }

  function renamePreset() {
    const name = customSelected ? draftName() : null;
    if (
      name !== null &&
      changePresets(() => renameCustomPreset(presetStorage, presetName, name))
    ) {
      setPresetName(name);
    }
  }

  function deletePreset() {
    if (!customSelected) {
      return;
    }
    changePresets(() => deleteCustomPreset(presetStorage, presetName));
    setPresetName(BUILT_IN_PRESETS[0].name);
  }

  function toggle(values: string[], value: string): string[] {
    return values.includes(value) ? values.filter((item) => item !== value) : [...values, value];
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    const problem = pixelSizeProblem(form);
    setFormProblem(problem);
    if (problem === null) {
      onStart(buildAcquisitionRequest(form));
    }
  }

  return (
    <form className="acquire-form" aria-label="Acquisition request" onSubmit={submit}>
      <fieldset>
        <legend>Presets</legend>
        <label>
          Preset
          <select value={presetName} onChange={(event) => setPresetName(event.target.value)}>
            <optgroup label="Built-in">
              {BUILT_IN_PRESETS.map((preset) => (
                <option key={preset.name}>{preset.name}</option>
              ))}
            </optgroup>
            {customPresets.length > 0 ? (
              <optgroup label="Saved">
                {customPresets.map((preset) => (
                  <option key={preset.name}>{preset.name}</option>
                ))}
              </optgroup>
            ) : null}
          </select>
        </label>
        <button
          type="button"
          disabled={selectedPreset === undefined}
          onClick={() => selectedPreset && update(applyPreset(form, selectedPreset))}
        >
          Apply preset
        </button>
        <label>
          Preset name
          <input value={presetDraftName} onChange={(event) => setPresetDraftName(event.target.value)} />
        </label>
        <button type="button" onClick={savePreset}>
          Save preset
        </button>
        <button type="button" disabled={!customSelected} onClick={renamePreset}>
          Rename preset
        </button>
        <button type="button" disabled={!customSelected} onClick={deletePreset}>
          Delete preset
        </button>
        {presetProblem ? (
          <p className="error-message" role="alert">
            {presetProblem}
          </p>
        ) : null}
      </fieldset>
      <fieldset>
        <legend>Sources</legend>
        <label className="choice">
          <input
            type="checkbox"
            checked={form.autoSources}
            onChange={(event) => update({ autoSources: event.target.checked })}
          />
          Auto (any compatible source)
        </label>
        {KNOWN_SOURCES.map((source) => (
          <label className="choice" key={source.value}>
            <input
              type="checkbox"
              disabled={form.autoSources}
              checked={form.sources.includes(source.value)}
              onChange={() => update({ sources: toggle(form.sources, source.value) })}
            />
            {source.label}
          </label>
        ))}
      </fieldset>

      <div className="acquire-targets">
        <label>
          Platforms (one per line)
          <textarea
            value={form.platforms}
            onChange={(event) => update({ platforms: event.target.value })}
            rows={3}
          />
        </label>
        <label>
          Games (one per line, empty for all)
          <textarea
            value={form.games}
            onChange={(event) => update({ games: event.target.value })}
            rows={3}
          />
        </label>
        <label>
          Regions (comma-separated, empty for any)
          <input value={form.regions} onChange={(event) => update({ regions: event.target.value })} />
        </label>
        <label>
          Languages (comma-separated, empty for any)
          <input
            value={form.languages}
            onChange={(event) => update({ languages: event.target.value })}
          />
        </label>
        <label>
          Minimum width (px, empty for any)
          <input
            inputMode="numeric"
            value={form.minWidth}
            onChange={(event) => update({ minWidth: event.target.value })}
          />
        </label>
        <label>
          Minimum height (px, empty for any)
          <input
            inputMode="numeric"
            value={form.minHeight}
            onChange={(event) => update({ minHeight: event.target.value })}
          />
        </label>
        <label>
          Retention policy
          <select
            value={form.retention}
            onChange={(event) => update({ retention: event.target.value as RetentionPolicy })}
          >
            <option value="keep_everything">Keep everything</option>
            <option value="keep_best_per_type">Keep best per type</option>
          </select>
        </label>
      </div>

      <fieldset className="asset-types">
        <legend>Asset types</legend>
        {ASSET_TYPE_FAMILIES.map((family) => (
          <div className="asset-family" key={family.value}>
            <label className="choice family">
              <input
                type="checkbox"
                aria-label={`All ${family.label}`}
                checked={form.assetTypes.includes(family.value)}
                onChange={() => update({ assetTypes: toggle(form.assetTypes, family.value) })}
              />
              {family.label}
            </label>
            {family.types.map((type) => (
              <label className="choice" key={type.value}>
                <input
                  type="checkbox"
                  checked={form.assetTypes.includes(type.value)}
                  onChange={() => update({ assetTypes: toggle(form.assetTypes, type.value) })}
                />
                {type.label}
              </label>
            ))}
          </div>
        ))}
      </fieldset>

      {formProblem ? (
        <p className="error-message" role="alert">
          {formProblem}
        </p>
      ) : null}
      {planProblem ? (
        <p className="error-message" role="alert">
          {planProblem}
        </p>
      ) : null}
      {plan ? <PlanSummary plan={plan} /> : null}
      {onCheckPlan ? (
        <button type="button" onClick={() => void checkPlan()}>
          Check plan
        </button>
      ) : null}
      <button type="submit" disabled={starting}>
        {starting ? "Starting…" : "Start acquisition"}
      </button>
    </form>
  );
}

/** Which Sources acquire each requested Asset Type, and why the others are left out. */
function PlanSummary({ plan }: { plan: AcquisitionPlan }) {
  return (
    <section className="acquisition-plan" aria-label="Acquisition plan">
      <ul>
        {plan.coverage.map((covered) => (
          <li key={covered.selector}>
            {assetTypeLabel(covered.selector)}: {covered.sources.map(sourceLabel).join(", ")}
          </li>
        ))}
        {plan.excluded.map((excluded) => (
          <li key={excluded.source_id}>
            {sourceLabel(excluded.source_id)} left out: {excluded.reason}
          </li>
        ))}
      </ul>
    </section>
  );
}

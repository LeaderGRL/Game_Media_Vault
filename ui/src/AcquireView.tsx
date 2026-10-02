import { FormEvent, useState } from "react";

import {
  ASSET_TYPE_FAMILIES,
  AcquisitionForm,
  AcquisitionRequestDraft,
  KNOWN_SOURCES,
  RetentionPolicy,
  buildAcquisitionRequest,
  emptyAcquisitionForm,
} from "./acquisition";

interface AcquireViewProps {
  starting: boolean;
  onStart: (request: AcquisitionRequestDraft) => void;
}

/** Compact Acquisition Request builder; the backend validates the submitted draft. */
export function AcquireView({ starting, onStart }: AcquireViewProps) {
  const [form, setForm] = useState<AcquisitionForm>(emptyAcquisitionForm);

  function update(change: Partial<AcquisitionForm>) {
    setForm((current) => ({ ...current, ...change }));
  }

  function toggle(values: string[], value: string): string[] {
    return values.includes(value) ? values.filter((item) => item !== value) : [...values, value];
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    onStart(buildAcquisitionRequest(form));
  }

  return (
    <form className="acquire-form" aria-label="Acquisition request" onSubmit={submit}>
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

      <button type="submit" disabled={starting}>
        {starting ? "Starting…" : "Start acquisition"}
      </button>
    </form>
  );
}

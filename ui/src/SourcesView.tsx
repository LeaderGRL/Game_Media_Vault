import { useState } from "react";

import {
  CredentialState,
  SourceDescription,
  SourceFailure,
  SourceFailureSummary,
  assetTypeLabel,
  sourceLabel,
} from "./acquisition";
import { errorMessage } from "./types";

interface SourcesViewProps {
  /** The registered Sources, or `null` while they are read or after reading them failed. */
  sources: SourceDescription[] | null;
  /** Why the Sources could not be read, if they could not. */
  error?: string | null;
  /** The failures the loaded vault recorded by Source; `null` without a vault or a reading. */
  failures?: SourceFailureSummary[] | null;
  /** Enables or disables a Source on this machine; without it, the state is only shown. */
  onSetEnabled?: (sourceId: string, enabled: boolean) => Promise<void>;
  /** Stores the API key of a Source on this machine; without it, no key is asked for. */
  onSetApiKey?: (sourceId: string, key: string) => Promise<void>;
  /** Forgets the API key this machine stores for a Source. */
  onClearApiKey?: (sourceId: string) => Promise<void>;
}

/** The registered Sources, described from the capabilities planning uses. */
export function SourcesView({
  sources,
  error = null,
  failures = null,
  onSetEnabled,
  onSetApiKey,
  onClearApiKey,
}: SourcesViewProps) {
  // The Source whose state is changing, one at a time, and why the last change failed.
  const [changing, setChanging] = useState<string | null>(null);
  const [changeError, setChangeError] = useState<string | null>(null);

  /** Makes one change to a Source, and says whether it succeeded. */
  async function change(sourceId: string, action: () => Promise<void>): Promise<boolean> {
    setChanging(sourceId);
    setChangeError(null);
    try {
      await action();
      return true;
    } catch (reason) {
      setChangeError(errorMessage(reason));
      return false;
    } finally {
      setChanging(null);
    }
  }

  async function setEnabled(sourceId: string, enabled: boolean) {
    if (onSetEnabled !== undefined) {
      await change(sourceId, () => onSetEnabled(sourceId, enabled));
    }
  }

  if (error !== null) {
    return (
      <p className="error-message" role="alert">
        {error}
      </p>
    );
  }
  if (sources === null) {
    return <p className="hint">Reading the registered Sources…</p>;
  }
  return (
    <div className="sources">
      {changeError !== null ? (
        <p className="error-message" role="alert">
          {changeError}
        </p>
      ) : null}
      {sources.map((source) => {
        const name = sourceLabel(source.source_id);
        const summary = failures?.find((failure) => failure.source_id === source.source_id);
        return (
          <section className="source" aria-label={name} key={source.source_id}>
            <h2>{name}</h2>
            <dl>
              <dt>On this machine</dt>
              <dd>
                <label className="choice">
                  <input
                    type="checkbox"
                    checked={source.enabled}
                    disabled={onSetEnabled === undefined || changing !== null}
                    onChange={() => void setEnabled(source.source_id, !source.enabled)}
                  />
                  Enabled on this machine
                </label>
                {source.enabled ? null : (
                  <p className="hint">Takes no part in acquisitions on this machine</p>
                )}
              </dd>
              {source.credential === "not_needed" ? null : (
                <ApiKeyField
                  name={name}
                  credential={source.credential}
                  busy={changing !== null}
                  onStore={
                    onSetApiKey === undefined
                      ? undefined
                      : (key) => change(source.source_id, () => onSetApiKey(source.source_id, key))
                  }
                  onForget={
                    onClearApiKey === undefined
                      ? undefined
                      : () => change(source.source_id, () => onClearApiKey(source.source_id))
                  }
                />
              )}
              <dt>Acquires</dt>
              <dd>{source.asset_types.map(assetTypeLabel).join(", ")}</dd>
              <dt>Acquisition method</dt>
              <dd>
                {source.direct_media_download
                  ? "Downloads media directly"
                  : "Cannot download media directly"}
              </dd>
              {failures !== null ? (
                <>
                  <dt>Recent failures</dt>
                  <dd>
                    {summary === undefined ? (
                      "No failure recorded"
                    ) : (
                      <>
                        <span>
                          {summary.failures}{" "}
                          {summary.failures === 1 ? "failure recorded" : "failures recorded"}
                        </span>
                        <ul className="source-failures">
                          {summary.latest.map((failure) => (
                            <li key={failure.sequence}>{describeFailure(failure)}</li>
                          ))}
                        </ul>
                      </>
                    )}
                  </dd>
                </>
              ) : null}
            </dl>
          </section>
        );
      })}
    </div>
  );
}

const CREDENTIAL_STATES: Record<Exclude<CredentialState, "not_needed">, string> = {
  missing: "Needs an API key, which this machine does not store",
  stored: "Stored in this machine's credential store",
  unreadable: "This machine's credential store could not be read",
};

interface ApiKeyFieldProps {
  name: string;
  credential: Exclude<CredentialState, "not_needed">;
  busy: boolean;
  /** Stores the key typed in, and says whether it did. */
  onStore?: (key: string) => Promise<boolean>;
  onForget?: () => Promise<boolean>;
}

/** Whether this machine stores the API key a Source needs, never the key itself. */
function ApiKeyField({ name, credential, busy, onStore, onForget }: ApiKeyFieldProps) {
  const [key, setKey] = useState("");

  async function store() {
    if (onStore !== undefined && (await onStore(key))) {
      // A key that failed to store stays for another try.
      setKey("");
    }
  }

  return (
    <>
      <dt>API key</dt>
      <dd>
        <p>{CREDENTIAL_STATES[credential]}</p>
        {onStore === undefined ? null : (
          <form
            className="api-key"
            onSubmit={(event) => {
              event.preventDefault();
              void store();
            }}
          >
            <input
              type="password"
              autoComplete="off"
              aria-label={`API key for ${name}`}
              value={key}
              disabled={busy}
              onChange={(event) => setKey(event.target.value)}
            />
            <button type="submit" disabled={busy || key.trim() === ""}>
              Store key
            </button>
            {credential === "stored" && onForget !== undefined ? (
              <button type="button" disabled={busy} onClick={() => void onForget()}>
                Forget key
              </button>
            ) : null}
          </form>
        )}
      </dd>
    </>
  );
}

function describeFailure(failure: SourceFailure) {
  // Recorded times read the same on every machine.
  const recorded = new Date(failure.recorded_at * 1000).toISOString().slice(0, 16);
  return `${recorded.replace("T", " ")} UTC · ${failure.stage} · run ${failure.run_id}: ${failure.message}`;
}

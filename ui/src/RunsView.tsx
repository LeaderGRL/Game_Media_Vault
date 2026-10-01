import type { AcquisitionRun, AcquisitionRunStatus } from "./acquisition";

interface RunsViewProps {
  runs: AcquisitionRun[];
  /** Runs with a pause, resume or cancel request in flight. */
  busyRunIds: ReadonlySet<number>;
  /** Runs being executed; they can still be paused or cancelled. */
  executingRunIds: ReadonlySet<number>;
  onExecute: (runId: number) => void;
  onPause: (runId: number) => void;
  onResume: (runId: number) => void;
  onCancel: (runId: number) => void;
}

const STATUS_LABELS: Record<AcquisitionRunStatus, string> = {
  running: "Running",
  paused: "Paused",
  cancelled: "Cancelled",
  completed: "Completed",
};

/** Persisted Acquisition Runs with the actions their status allows. */
export function RunsView({
  runs,
  busyRunIds,
  executingRunIds,
  onExecute,
  onPause,
  onResume,
  onCancel,
}: RunsViewProps) {
  if (runs.length === 0) {
    return (
      <section className="empty-state" aria-live="polite">
        <h2>No acquisition runs</h2>
        <p>Start an acquisition from the Acquire view.</p>
      </section>
    );
  }

  return (
    <section className="run-list" aria-label="Acquisition runs">
      {runs.map((run) => {
        const busy = busyRunIds.has(run.id);
        const executing = executingRunIds.has(run.id);
        const title = `Run #${run.id}`;
        return (
          <article className="run-card" key={run.id} aria-label={title}>
            <div className="run-heading">
              <h2>{title}</h2>
              <span className={`run-status run-status-${run.status}`}>
                {STATUS_LABELS[run.status]}
              </span>
            </div>
            <p className="release-line">{describeRequest(run)}</p>
            <p className="run-counts">
              {run.queued_work} queued · {run.awaiting_review_work} awaiting review ·{" "}
              {run.completed_work} completed
            </p>
            <div className="run-actions">
              {run.status === "running" ? (
                <>
                  <button
                    type="button"
                    disabled={busy || executing}
                    onClick={() => onExecute(run.id)}
                  >
                    {executing ? "Executing…" : "Execute"}
                  </button>
                  <button type="button" disabled={busy} onClick={() => onPause(run.id)}>
                    Pause
                  </button>
                </>
              ) : null}
              {run.status === "paused" ? (
                <button type="button" disabled={busy} onClick={() => onResume(run.id)}>
                  Resume
                </button>
              ) : null}
              {run.status === "running" || run.status === "paused" ? (
                <button type="button" disabled={busy} onClick={() => onCancel(run.id)}>
                  Cancel
                </button>
              ) : null}
            </div>
          </article>
        );
      })}
    </section>
  );
}

function describeRequest(run: AcquisitionRun): string {
  const sources =
    run.request.sources.mode === "auto" ? "Auto" : run.request.sources.values.join(", ");
  return [sources, run.request.platforms.join(", ")].filter((part) => part.length > 0).join(" · ");
}

import { useState } from "react";

import type { AcquisitionRun } from "./acquisition";
import { assetTypeLabel } from "./acquisition";
import { ProgressBar } from "./controls";
import { Icon } from "./icons";
import { thumbnailOf } from "./ReleaseDetail";
import { describeRunRequest, runProgress, runTitle } from "./runProgress";
import type { LatestMedium } from "./types";

/** A download being prepared: its run starting, which fetches its game list and plans it. */
export interface PreparingDownload {
  key: number;
  /** What it downloads, such as its consoles. */
  title: string;
}

interface ActivityViewProps {
  runs: AcquisitionRun[];
  /** Downloads being prepared: their game list fetched, their plan checked. */
  preparing: PreparingDownload[];
  /** Runs executing now. */
  executingRunIds: ReadonlySet<number>;
  /** Runs waiting for the one executing to finish. */
  waitingRunIds: ReadonlySet<number>;
  /** Runs with a pause, resume or cancel request in flight. */
  busyRunIds: ReadonlySet<number>;
  /** The media the vault retained last, newest first. */
  latest: LatestMedium[];
  objectUrl: (objectHash: string) => string;
  onContinue: (runId: number) => void;
  onPause: (runId: number) => void;
  onResume: (runId: number) => void;
  onCancel: (runId: number) => void;
}

type Phase = "downloading" | "waiting" | "stopped" | "paused" | "completed" | "cancelled";

const PHASES: Record<Phase, { label: string; tone: string }> = {
  downloading: { label: "Downloading", tone: "live" },
  waiting: { label: "Waiting", tone: "waiting" },
  stopped: { label: "Stopped", tone: "stopped" },
  paused: { label: "Paused", tone: "paused" },
  completed: { label: "Completed", tone: "done" },
  cancelled: { label: "Cancelled", tone: "" },
};

/** Downloads under way, waiting or done, with their live progress and the media arriving. */
export function ActivityView({
  runs,
  preparing,
  executingRunIds,
  waitingRunIds,
  busyRunIds,
  latest,
  objectUrl,
  onContinue,
  onPause,
  onResume,
  onCancel,
}: ActivityViewProps) {
  const newestFirst = [...runs].sort((a, b) => b.id - a.id);
  if (newestFirst.length === 0 && preparing.length === 0) {
    return (
      <section className="empty-state" aria-live="polite">
        <span className="empty-icon">
          <Icon name="activity" size={26} />
        </span>
        <h2>No downloads yet</h2>
        <p>Choose consoles in Download: their progress shows here as media arrive.</p>
      </section>
    );
  }

  return (
    <>
      {latest.length > 0 ? (
        <section className="latest-strip" aria-label="Latest media">
          <h2>
            <Icon name="sparkle" size={18} />
            Just arrived
          </h2>
          <div className="latest-row">
            {latest.map((medium) => (
              <LatestItem key={medium.asset.asset_id} medium={medium} objectUrl={objectUrl} />
            ))}
          </div>
        </section>
      ) : null}
      <section className="run-list" aria-label="Downloads">
        {preparing.map((download) => (
          <article className="card run-card" key={download.key} aria-label={download.title}>
            <div className="run-heading">
              <h2>{download.title}</h2>
              <span className="status-pill waiting">Preparing</span>
            </div>
            <ProgressBar
              label="Preparing"
              value={null}
              detail="Fetching the game list and checking the Sources"
            />
          </article>
        ))}
        {newestFirst.map((run) => {
          const phase = phaseOf(run, executingRunIds, waitingRunIds);
          const progress = runProgress(run);
          const busy = busyRunIds.has(run.id);
          const title = runTitle(run);
          return (
            <article className="card run-card" key={run.id} aria-label={`${title} · Run #${run.id}`}>
              <div className="run-heading">
                <div>
                  <h2>{title}</h2>
                  <p className="run-subtitle">{describeRunRequest(run)}</p>
                </div>
                <span className={`status-pill ${PHASES[phase].tone}`}>{PHASES[phase].label}</span>
                <div className="run-actions">
                  {phase === "stopped" ? (
                    <button
                      type="button"
                      className="primary"
                      disabled={busy}
                      onClick={() => onContinue(run.id)}
                    >
                      <Icon name="play" size={16} />
                      Continue
                    </button>
                  ) : null}
                  {phase === "downloading" || phase === "waiting" ? (
                    <button type="button" disabled={busy} onClick={() => onPause(run.id)}>
                      <Icon name="pause" size={16} />
                      Pause
                    </button>
                  ) : null}
                  {run.status === "paused" ? (
                    <button
                      type="button"
                      className="primary"
                      disabled={busy}
                      onClick={() => onResume(run.id)}
                    >
                      <Icon name="play" size={16} />
                      Resume
                    </button>
                  ) : null}
                  {run.status === "running" || run.status === "paused" ? (
                    <button
                      type="button"
                      className="ghost danger"
                      disabled={busy}
                      onClick={() => onCancel(run.id)}
                    >
                      Cancel
                    </button>
                  ) : null}
                </div>
              </div>
              {progress.sources > 0 && (progress.searching || phase !== "completed") ? (
                <ProgressBar
                  label="Search"
                  tone="search"
                  value={progress.searched}
                  detail={`${progress.sourcesDone} of ${progress.sources} Sources done`}
                />
              ) : null}
              <ProgressBar
                label="Downloads"
                value={progress.downloaded}
                detail={`${progress.settled} of ${progress.settled + progress.toDownload} done`}
              />
              <div className="run-stats">
                <Stat value={progress.acquired} label="acquired" />
                <Stat value={progress.toDownload} label="to download" />
                <Stat value={progress.toReview} label="to review" />
                <Stat value={progress.notFound} label="not found" />
                {progress.skipped > 0 ? <Stat value={progress.skipped} label="skipped" /> : null}
                {progress.rejected > 0 ? <Stat value={progress.rejected} label="rejected" /> : null}
              </div>
            </article>
          );
        })}
      </section>
    </>
  );
}

function phaseOf(
  run: AcquisitionRun,
  executing: ReadonlySet<number>,
  waiting: ReadonlySet<number>,
): Phase {
  if (run.status === "paused") {
    return "paused";
  }
  if (run.status === "completed") {
    return "completed";
  }
  if (run.status === "cancelled") {
    return "cancelled";
  }
  if (executing.has(run.id)) {
    return "downloading";
  }
  return waiting.has(run.id) ? "waiting" : "stopped";
}

function Stat({ value, label }: { value: number; label: string }) {
  return (
    <div className="run-stat">
      <strong>{value}</strong>
      <span>{label}</span>
    </div>
  );
}

/**
 * A medium that just arrived: its thumbnail, else its original when it is an image, else a
 * placeholder, each tried once the one before it cannot be shown.
 */
function LatestItem({
  medium,
  objectUrl,
}: {
  medium: LatestMedium;
  objectUrl: (objectHash: string) => string;
}) {
  const asset = medium.asset;
  const [failed, setFailed] = useState<ReadonlySet<string>>(() => new Set());
  const shown = [
    thumbnailOf(asset)?.object_hash,
    asset.media_type.startsWith("image/") ? asset.object_hash : undefined,
  ].find((hash) => hash !== undefined && !failed.has(hash));
  const description = `${assetTypeLabel(asset.asset_type)} of ${medium.game_title}`;
  return (
    <div className="latest-item">
      {shown === undefined ? (
        <div className="media-placeholder" aria-label={description} role="img">
          <Icon name="image" size={26} />
        </div>
      ) : (
        <img
          key={shown}
          src={objectUrl(shown)}
          alt={description}
          loading="lazy"
          onError={() => setFailed((current) => new Set(current).add(shown))}
        />
      )}
      <span title={medium.game_title}>{medium.game_title}</span>
      <span className="hint">{assetTypeLabel(asset.asset_type)}</span>
    </div>
  );
}

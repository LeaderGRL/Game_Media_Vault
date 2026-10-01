import { useEffect, useRef, useState } from "react";

import { previewMediaType } from "./types";
import type { ReviewDecision, ReviewItem } from "./types";

interface ReviewViewProps {
  items: ReviewItem[];
  resolvingIds: ReadonlySet<number>;
  onResolve: (reviewItemId: number, decision: ReviewDecision) => void;
  onLoadPreview: (reviewItemId: number) => Promise<ArrayBuffer>;
}

export function ReviewView({ items, resolvingIds, onResolve, onLoadPreview }: ReviewViewProps) {
  if (items.length === 0) {
    return (
      <section className="empty-state" aria-live="polite">
        <h2>No review items</h2>
        <p>Medium-confidence matches will appear here with their evidence.</p>
      </section>
    );
  }

  return (
    <section className="review-list" aria-label="Review items">
      {items.map((item) => {
        const busy = resolvingIds.has(item.id);
        const closed = item.status !== "pending" && item.status !== "deferred";
        return (
          <article className="review-card" key={item.id}>
            <div className="review-heading">
              <div>
                <span className="review-kicker">Review #{item.id}</span>
                <h2>{item.candidate.game_title}</h2>
                <p className="release-line">
                  {item.candidate.platform} · {item.candidate.region} · {item.candidate.edition_name}
                </p>
              </div>
              <span className="review-status">{formatStatus(item.status, item.decision)}</span>
            </div>

            <div className="review-source">
              <CandidatePreview
                key={`${item.id}:${item.candidate.source_url}`}
                item={item}
                enabled={!closed}
                onLoadPreview={onLoadPreview}
              />
              <span className="detail-label">Source evidence</span>
              <strong>
                {item.candidate.source_id}
                {item.candidate.source_asset_label ? ` · ${item.candidate.source_asset_label}` : ""}
              </strong>
              <code>{item.candidate.source_url}</code>
            </div>

            <div className="review-matches">
              {item.competing_matches.map((match) => (
                <section className="review-match" key={match.release_edition_id}>
                  <div className="review-match-heading">
                    <div>
                      <h3>{match.edition_name}</h3>
                      <p>
                        {match.platform} · {match.region} · release #{match.release_edition_id}
                      </p>
                    </div>
                    <span className="score-badge">Score {match.score}</span>
                  </div>

                  <div className="evidence-list" aria-label={`Score explanation for ${match.edition_name}`}>
                    {match.evidence.map((evidence, index) => (
                      <div className="evidence-row" key={`${evidence.signal}-${index}`}>
                        <strong>
                          {formatSignal(evidence.signal)}: {formatDelta(evidence.score_delta)}
                        </strong>
                        <span>
                          {evidence.candidate_value} → {evidence.release_value}
                        </span>
                      </div>
                    ))}
                  </div>

                  {match.assertions.length > 0 ? (
                    <div
                      className="evidence-list"
                      aria-label={`Source assertions for ${match.edition_name}`}
                    >
                      {match.assertions.map((assertion, index) => (
                        <div
                          className="evidence-row"
                          key={`${assertion.source_id}-${assertion.field}-${assertion.value}-${index}`}
                        >
                          <strong>{assertion.source_id}</strong>
                          <span>
                            {assertion.field}: {assertion.value}
                          </span>
                          <code>{assertion.source_location}</code>
                        </div>
                      ))}
                    </div>
                  ) : null}

                  <button
                    type="button"
                    disabled={busy || closed}
                    onClick={() =>
                      onResolve(item.id, {
                        decision: "accept",
                        release_edition_id: match.release_edition_id,
                      })
                    }
                  >
                    Accept {match.edition_name}
                  </button>
                </section>
              ))}
            </div>

            <div className="review-actions">
              <button
                type="button"
                disabled={busy || closed}
                onClick={() => onResolve(item.id, { decision: "reject" })}
              >
                Reject candidate
              </button>
              <button
                type="button"
                disabled={busy || closed}
                onClick={() => onResolve(item.id, { decision: "defer" })}
              >
                Defer review
              </button>
            </div>
          </article>
        );
      })}
    </section>
  );
}

function CandidatePreview({
  item,
  enabled,
  onLoadPreview,
}: {
  item: ReviewItem;
  enabled: boolean;
  onLoadPreview: (reviewItemId: number) => Promise<ArrayBuffer>;
}) {
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [previewFailed, setPreviewFailed] = useState(false);
  const [previewLoading, setPreviewLoading] = useState(false);
  const active = useRef(true);
  const enabledRef = useRef(enabled);

  useEffect(() => {
    // StrictMode runs this effect twice, so the setup must undo the previous cleanup.
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);

  useEffect(() => {
    enabledRef.current = enabled;
    if (!enabled) {
      // A closed review keeps its card in the history; release the preview bytes.
      setPreviewUrl(null);
    }
  }, [enabled]);

  useEffect(() => {
    return () => {
      if (previewUrl !== null) {
        URL.revokeObjectURL(previewUrl);
      }
    };
  }, [previewUrl]);

  async function loadPreview() {
    if (previewLoading || previewUrl !== null) {
      return;
    }
    setPreviewLoading(true);
    setPreviewFailed(false);
    try {
      const preview = await onLoadPreview(item.id);
      if (!active.current || !enabledRef.current) {
        return;
      }
      const objectUrl = URL.createObjectURL(
        new Blob([preview], {
          type: previewMediaType(new Uint8Array(preview), item.candidate.original_filename),
        }),
      );
      setPreviewUrl(objectUrl);
    } catch {
      if (active.current) {
        setPreviewFailed(true);
      }
    } finally {
      if (active.current) {
        setPreviewLoading(false);
      }
    }
  }

  if (!enabled) {
    return (
      <div className="review-preview review-preview-status">
        Preview not loaded for closed review
      </div>
    );
  }

  if (previewUrl === null) {
    return (
      <div className="review-preview review-preview-status">
        <button type="button" disabled={previewLoading} onClick={() => void loadPreview()}>
          {previewLoading ? "Loading preview…" : "Load preview"}
        </button>
        {previewFailed ? <span role="status">Preview unavailable</span> : null}
      </div>
    );
  }

  return (
    <img
      className="review-preview"
      src={previewUrl}
      alt={`${item.candidate.game_title} box front candidate`}
    />
  );
}

function formatStatus(status: ReviewItem["status"], decision: ReviewDecision | null) {
  if (status === "accepted" && decision?.decision === "accept") {
    return `Accepted · release #${decision.release_edition_id}`;
  }
  if (status === "rejected") {
    return "Rejected";
  }
  if (status === "deferred") {
    return "Deferred";
  }
  if (status === "auto_resolved") {
    return "Auto-resolved";
  }
  if (status === "superseded") {
    return "Superseded";
  }
  return "Pending";
}

function formatSignal(signal: string) {
  return signal.charAt(0).toUpperCase() + signal.slice(1);
}

function formatDelta(delta: number) {
  return delta >= 0 ? `+${delta}` : String(delta);
}

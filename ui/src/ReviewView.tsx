import type { ReviewDecision, ReviewItem } from "./types";

interface ReviewViewProps {
  items: ReviewItem[];
  resolvingIds: ReadonlySet<number>;
  onResolve: (reviewItemId: number, decision: ReviewDecision) => void;
}

export function ReviewView({ items, resolvingIds, onResolve }: ReviewViewProps) {
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
        const closed = [
          "processing",
          "accepted",
          "applied",
          "rejected",
          "auto_resolved",
          "superseded",
        ].includes(item.status);
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
              <img
                className="review-preview"
                src={item.candidate.source_url}
                alt={`${item.candidate.game_title} box front candidate`}
                loading="lazy"
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

function formatStatus(status: ReviewItem["status"], decision: ReviewDecision | null) {
  if (status === "accepted" && decision?.decision === "accept") {
    return `Accepted · release #${decision.release_edition_id}`;
  }
  if (status === "applied" && decision?.decision === "accept") {
    return `Applied · release #${decision.release_edition_id}`;
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
  if (status === "processing") {
    return "Processing";
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

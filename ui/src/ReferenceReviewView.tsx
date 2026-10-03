import type { ReferenceReviewEdition, ReferenceReviewItem } from "./types";

interface ReferenceReviewViewProps {
  items: ReferenceReviewItem[];
  decidingIds: ReadonlySet<number>;
  onLink: (itemId: number, releaseEditionId: number) => void;
  onKeepApart: (itemId: number) => void;
}

/** The reference records awaiting a human to tell which edition, if any, they describe. */
export function ReferenceReviewView({ items, decidingIds, onLink, onKeepApart }: ReferenceReviewViewProps) {
  return (
    <section className="review-list" aria-label="Reference review items">
      {items.map((item) => {
        const busy = decidingIds.has(item.id);
        const own = editionOf(item, item.release_edition_id);
        const label = `Reference review #${item.id}`;
        return (
          <article className="review-card" key={item.id} aria-label={label}>
            <div className="review-heading">
              <div>
                <span className="review-kicker">{label}</span>
                <h2>{own.game_title}</h2>
                <p className="release-line">
                  {own.platform} · {own.region} · {own.edition_name}
                </p>
              </div>
              <span className="review-status">{formatEvidence(item.evidence)}</span>
            </div>

            <div className="review-source">
              <span className="detail-label">Reference record</span>
              <strong>
                {item.source_id} · {item.source_record}
              </strong>
              <RecordList edition={own} except={item} />
            </div>

            <ul className="review-matches" aria-label="Candidate editions">
              {item.candidates.map((candidateId) => {
                const candidate = editionOf(item, candidateId);
                return (
                  <li className="review-match" key={candidateId}>
                    <div className="review-match-heading">
                      <div>
                        <h3>{candidate.game_title}</h3>
                        <p>
                          {candidate.platform} · {candidate.region} · {candidate.edition_name} ·
                          release #{candidateId}
                        </p>
                      </div>
                    </div>
                    <RecordList edition={candidate} />
                    <button type="button" disabled={busy} onClick={() => onLink(item.id, candidateId)}>
                      Same release as {candidate.game_title}
                    </button>
                  </li>
                );
              })}
            </ul>

            <div className="review-actions">
              <button type="button" disabled={busy} onClick={() => onKeepApart(item.id)}>
                Distinct release
              </button>
            </div>
          </article>
        );
      })}
    </section>
  );
}

/** The records sources hold on `edition`, but for the one of `except`. */
function RecordList({
  edition,
  except,
}: {
  edition: ReferenceReviewEdition;
  except?: ReferenceReviewItem;
}) {
  const records = edition.records.filter(
    (record) =>
      except === undefined ||
      record.source_id !== except.source_id ||
      record.source_record !== except.source_record,
  );
  if (records.length === 0) {
    return null;
  }
  return (
    <div className="evidence-list" aria-label={`Records on release #${edition.release_edition_id}`}>
      {records.map((record) => (
        <div className="evidence-row" key={`${record.source_id}-${record.source_record}`}>
          <strong>{record.title}</strong>
          <span>
            {record.source_id} · {record.source_record}
          </span>
        </div>
      ))}
    </div>
  );
}

/** The edition `id` as the item describes it, or its id alone once the catalog lost it. */
function editionOf(item: ReferenceReviewItem, id: number): ReferenceReviewEdition {
  return (
    item.editions.find((edition) => edition.release_edition_id === id) ?? {
      release_edition_id: id,
      game_title: `Release #${id}`,
      platform: "",
      region: "",
      edition_name: "",
      records: [],
    }
  );
}

function formatEvidence(evidence: ReferenceReviewItem["evidence"]): string {
  return evidence === "sha1" ? "Same dumps" : "Same title";
}

import type { LibraryEntry } from "./types";

interface LibraryViewProps {
  entries: LibraryEntry[];
}

export function LibraryView({ entries }: LibraryViewProps) {
  if (entries.length === 0) {
    return (
      <section className="empty-state" aria-live="polite">
        <h2>Library is empty</h2>
        <p>Import a Box Front from the CLI, then load the same vault here.</p>
      </section>
    );
  }

  return (
    <section className="library" aria-label="Library assets">
      {entries.map((entry) => (
        <article className="asset-row" key={entry.asset_id}>
          <div className="asset-copy">
            <h2>{entry.game_title}</h2>
            <p className="release-line">
              {entry.platform} · {entry.region} · {entry.edition_name}
            </p>
          </div>

          <div className="asset-details">
            <div>
              <span className="detail-label">Asset</span>
              <strong>Box Front</strong>
            </div>
            <div>
              <span className="detail-label">File</span>
              <strong>{entry.original_filename}</strong>
            </div>
            <div>
              <span className="detail-label">Size</span>
              <strong>{formatBytes(entry.byte_len)}</strong>
            </div>
          </div>

          <div className="provenance">
            <span className="detail-label">Provenance</span>
            {entry.provenance.map((source) => (
              <code key={`${source.source_kind}:${source.source_location}`}>
                {source.source_location}
              </code>
            ))}
          </div>
        </article>
      ))}
    </section>
  );
}

function formatBytes(byteLength: number) {
  if (byteLength < 1024) {
    return `${byteLength} B`;
  }
  return `${(byteLength / 1024).toFixed(1)} KiB`;
}

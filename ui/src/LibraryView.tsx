import type { LibraryEntry } from "./types";

interface LibraryViewProps {
  entries: LibraryEntry[];
}

export function LibraryView({ entries }: LibraryViewProps) {
  if (entries.length === 0) {
    return (
      <section className="empty-state" aria-live="polite">
        <h2>Library is empty</h2>
        <p>Import media or reference data, then load the same vault here.</p>
      </section>
    );
  }

  return (
    <section className="library" aria-label="Library releases">
      {entries.map((entry) => (
        <article className="asset-row" key={entry.release_edition_id}>
          <div className="asset-copy">
            <h2>{entry.game_title}</h2>
            <p className="release-line">
              {entry.platform} · {entry.region} · {entry.edition_name}
            </p>
          </div>

          <div className="release-assets">
            {entry.assets.length === 0 ? (
              <p className="no-assets">No media assets</p>
            ) : (
              entry.assets.map((asset) => (
                <div className="asset-record" key={asset.asset_id}>
                  <div className="asset-details">
                    <div>
                      <span className="detail-label">Asset</span>
                      <strong>{formatAssetType(asset.asset_type)}</strong>
                    </div>
                    <div>
                      <span className="detail-label">File</span>
                      <strong>{asset.original_filename}</strong>
                    </div>
                    <div>
                      <span className="detail-label">Size</span>
                      <strong>{formatBytes(asset.byte_len)}</strong>
                    </div>
                  </div>

                  <div className="provenance">
                    <span className="detail-label">Provenance</span>
                    {asset.provenance.map((source) => (
                      <code key={source.source_id + ":" + (source.source_asset_label ?? "") + ":" + source.source_location}>
                        {source.source_id}
                        {source.source_asset_label ? " · " + source.source_asset_label : ""}: {source.source_location}
                      </code>
                    ))}
                  </div>
                </div>
              ))
            )}
          </div>

          {entry.assertions.length > 0 ? (
            <div className="assertions">
              <span className="detail-label">Reference assertions</span>
              {entry.assertions.map((assertion, index) => (
                <code key={assertion.source_id + ":" + assertion.field + ":" + (assertion.qualifier ?? "") + ":" + assertion.value + ":" + index}>
                  {assertion.source_id} · {assertion.field}
                  {assertion.qualifier ? " (" + assertion.qualifier + ")" : ""}: {assertion.value} · {assertion.source_location}
                </code>
              ))}
            </div>
          ) : null}
        </article>
      ))}
    </section>
  );
}

function formatAssetType(assetType: string) {
  if (assetType === "box_front") {
    return "Box Front";
  }

  return assetType;
}

function formatBytes(byteLength: number) {
  if (byteLength < 1024) {
    return String(byteLength) + " B";
  }
  return (byteLength / 1024).toFixed(1) + " KiB";
}

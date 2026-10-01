import { useState } from "react";

import {
  filenameMediaType,
  type LibraryAsset,
  type LibraryEntry,
  type ReleaseAssertionField,
} from "./types";

interface LibraryViewProps {
  entries: LibraryEntry[];
  /** URL under which the desktop shell serves the original object with this hash. */
  objectUrl: (objectHash: string) => string;
}

export function LibraryView({ entries, objectUrl }: LibraryViewProps) {
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
            {entry.canonical_values.length > 0 ? (
              <dl className="canonical-values">
                {entry.canonical_values.map((canonical) => (
                  <div className="canonical-value" key={canonical.field + ":" + (canonical.qualifier ?? "")}>
                    <dt>
                      {formatField(canonical.field)}
                      {canonical.qualifier ? " (" + canonical.qualifier + ")" : ""}
                    </dt>
                    <dd>
                      <strong>{canonical.value}</strong>
                      <span className="canonical-support">
                        {canonical.confidence}% ·{" "}
                        {canonical.contributing.map((claim) => claim.source_id).join(", ")}
                      </span>
                      {canonical.conflicting.map((claim) => (
                        <span className="canonical-conflict" key={claim.source_id}>
                          Conflicts with {claim.source_id}: {claim.value}
                        </span>
                      ))}
                    </dd>
                  </div>
                ))}
              </dl>
            ) : null}
          </div>

          <div className="release-assets">
            {entry.assets.length === 0 ? (
              <p className="no-assets">No media assets</p>
            ) : (
              entry.assets.map((asset) => (
                <div className="asset-record" key={asset.asset_id}>
                  <AssetOriginal
                    asset={asset}
                    description={formatAssetType(asset.asset_type) + " of " + entry.game_title}
                    objectUrl={objectUrl}
                  />
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
                      {asset.width !== null && asset.height !== null ? (
                        <span className="pixel-size">
                          {asset.width} × {asset.height} px
                        </span>
                      ) : null}
                    </div>
                  </div>

                  <div className="provenance">
                    <span className="detail-label">Provenance</span>
                    {asset.provenance.map((source, index) => (
                      <code key={source.source_id + ":" + source.source_location + ":" + index}>
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

function formatField(field: ReleaseAssertionField) {
  return field.charAt(0).toUpperCase() + field.slice(1);
}

interface AssetOriginalProps {
  asset: LibraryAsset;
  description: string;
  objectUrl: (objectHash: string) => string;
}

/**
 * Thumbnail of an image original, or a placeholder when the vault cannot serve it or the view
 * cannot decode its format.
 */
function AssetOriginal({ asset, description, objectUrl }: AssetOriginalProps) {
  const [unavailable, setUnavailable] = useState(false);

  if (!isImageOriginal(asset)) {
    return null;
  }
  if (unavailable) {
    return <p className="asset-original unavailable">Preview unavailable</p>;
  }
  return (
    <img
      className="asset-original"
      src={objectUrl(asset.object_hash)}
      alt={description}
      loading="lazy"
      onError={() => setUnavailable(true)}
    />
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

/**
 * Whether an original is an image. Assets imported before media inspection (vaults upgraded
 * from schema version 2) still have unknown media, so their file name decides until the vault
 * is verified again.
 */
function isImageOriginal(asset: LibraryAsset) {
  const mediaType =
    asset.media_type === "application/octet-stream"
      ? filenameMediaType(asset.original_filename)
      : asset.media_type;
  return mediaType.startsWith("image/");
}

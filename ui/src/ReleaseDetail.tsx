import { useState } from "react";

import { ASSET_TYPE_FAMILIES, assetTypeLabel, attributionOf } from "./acquisition";
import { ExternalLink } from "./ExternalLink";
import { Icon } from "./icons";
import { PackagingModelPreview } from "./PackagingModelPreview";
import {
  filenameMediaType,
  type CoverageProfile,
  type CoverageStatus,
  type DocumentMetadata,
  type LibraryAsset,
  type LibraryEntry,
  type PackagingFamily,
  type PreferenceReason,
  type ReleaseAssertionField,
  type ReleaseCoverage,
} from "./types";

/** Longest edge of the thumbnails the Library shows instead of their originals. */
export const LIBRARY_THUMBNAIL_EDGE = 256;

interface ReleaseDetailProps {
  entry: LibraryEntry;
  /** URL under which the desktop shell serves the original object with this hash. */
  objectUrl: (objectHash: string) => string;
  /** Closes the panel showing the release; the close button shows only with it. */
  onClose?: () => void;
}

/**
 * Everything the vault knows of one release: its 3D box, coverage, every original grouped by
 * Asset Type with what it is and where it came from, its canonical values and assertions.
 */
export function ReleaseDetail({ entry, objectUrl, onClose }: ReleaseDetailProps) {
  const cover = coverOf(entry);
  return (
    <article className="release-detail" aria-label={entry.game_title}>
      <header className="drawer-header">
        <div className="drawer-cover">
          {cover ? (
            <img src={objectUrl(thumbnailOf(cover)?.object_hash ?? cover.object_hash)} alt="" />
          ) : (
            <div className="media-placeholder">
              <Icon name="gamepad" size={28} />
            </div>
          )}
        </div>
        <div>
          <h2>{entry.game_title}</h2>
          <p className="release-line">
            {entry.platform} · {entry.region} · {entry.edition_name}
          </p>
          <p className="release-line">
            {entry.assets.length} {entry.assets.length === 1 ? "medium" : "media"}
          </p>
        </div>
        {onClose ? (
          <button type="button" className="ghost icon-button" aria-label="Close" onClick={onClose}>
            <Icon name="close" size={18} />
          </button>
        ) : null}
      </header>
      <div className="drawer-body">
      <section className="drawer-section">
        <PackagingModelNote entry={entry} objectUrl={objectUrl} />
      </section>
      <section className="drawer-section">
        <h3>Coverage</h3>
        <ReleaseCoverageNote coverage={entry.coverage} />
      </section>
      <section className="drawer-section">
        <h3>Media</h3>
        {entry.assets.length === 0 ? (
          <p className="no-assets">No media assets</p>
        ) : (
          <div className="media-grid">
            {byAssetType(entry.assets).map((asset) => (
              <div className="asset-record" key={asset.asset_id}>
                <AssetOriginal
                  asset={asset}
                  description={assetTypeLabel(asset.asset_type) + " of " + entry.game_title}
                  objectUrl={objectUrl}
                />
                <div className="asset-details">
                  <div>
                    <span className="detail-label">Asset</span>
                    <strong>{assetTypeLabel(asset.asset_type)}</strong>
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
                  {asset.document ? <DocumentNote document={asset.document} /> : null}
                  <AssetPreferenceNote entry={entry} asset={asset} />
                </div>
                <div className="provenance">
                  <span className="detail-label">Provenance</span>
                  {asset.provenance.map((source, index) => {
                    const attribution = attributionOf(source.source_id);
                    return (
                      <code key={source.source_id + ":" + source.source_location + ":" + index}>
                        {source.source_id}
                        {source.source_asset_label ? " · " + source.source_asset_label : ""}:{" "}
                        {source.source_location}
                        {attribution ? (
                          <>
                            {" · via "}
                            <ExternalLink href={attribution.url}>{attribution.name}</ExternalLink>
                          </>
                        ) : null}
                      </code>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
        )}
      </section>
      {entry.canonical_values.length > 0 ? (
        <section className="drawer-section">
          <h3>Canonical values</h3>
          <dl className="canonical-values">
            {entry.canonical_values.map((canonical) => (
              <div
                className="canonical-value"
                key={canonical.field + ":" + (canonical.qualifier ?? "")}
              >
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
        </section>
      ) : null}
      {entry.assertions.length > 0 ? (
        <section className="drawer-section assertions">
          <span className="detail-label">Reference assertions</span>
          {entry.assertions.map((assertion, index) => (
            <code
              key={
                assertion.source_id +
                ":" +
                assertion.field +
                ":" +
                (assertion.qualifier ?? "") +
                ":" +
                assertion.value +
                ":" +
                index
              }
            >
              {assertion.source_id} · {assertion.field}
              {assertion.qualifier ? " (" + assertion.qualifier + ")" : ""}: {assertion.value} ·{" "}
              {assertion.source_location}
            </code>
          ))}
        </section>
      ) : null}
      </div>
    </article>
  );
}

/**
 * The original a release is shown by: its preferred Box Front, else its first image in taxonomy
 * order, if it has one.
 */
export function coverOf(entry: LibraryEntry): LibraryAsset | undefined {
  const preferredFront = entry.preferred_assets.find(
    (preferred) => preferred.asset_type === "box_front",
  )?.asset_id;
  // A document's thumbnail is its first page, never a cover.
  const images = byAssetType(entry.assets).filter(isImageOriginal);
  return (
    images.find((asset) => asset.asset_id === preferredFront) ??
    images.find((asset) => asset.asset_type === "box_front") ??
    images[0]
  );
}

function formatField(field: ReleaseAssertionField) {
  return field.charAt(0).toUpperCase() + field.slice(1);
}

/** What a PDF original says of itself: its pages, version and encryption, then its title and author. */
function DocumentNote({ document }: { document: DocumentMetadata }) {
  const pages =
    document.page_count === null
      ? null
      : `${document.page_count} ${document.page_count === 1 ? "page" : "pages"}`;
  const summary = [pages, `PDF ${document.version}`, document.encrypted ? "encrypted" : null]
    .filter(Boolean)
    .join(" · ");
  const credit = [document.title, document.author].filter(Boolean).join(" · ");
  return (
    <div>
      <span className="detail-label">Document</span>
      <strong>{summary}</strong>
      {credit ? <span className="pixel-size">{credit}</span> : null}
    </div>
  );
}

interface AssetOriginalProps {
  asset: LibraryAsset;
  description: string;
  objectUrl: (objectHash: string) => string;
}

/**
 * The Library thumbnail of an image original, else the original itself, or a player for a video
 * original, or a placeholder when the vault cannot serve them or the view cannot decode them.
 */
export function AssetOriginal({ asset, description, objectUrl }: AssetOriginalProps) {
  const [failedHashes, setFailedHashes] = useState<ReadonlySet<string>>(() => new Set());
  if (PLAYABLE_VIDEO_TYPES.includes(asset.media_type)) {
    if (failedHashes.has(asset.object_hash)) {
      return <p className="asset-original unavailable">Preview unavailable</p>;
    }
    return (
      <video
        className="asset-original"
        src={objectUrl(asset.object_hash)}
        aria-label={description}
        controls
        preload="metadata"
        onError={() => setFailedHashes((failed) => new Set(failed).add(asset.object_hash))}
      />
    );
  }
  const thumbnail = thumbnailOf(asset);

  if (thumbnail === undefined && !isImageOriginal(asset)) {
    return null;
  }
  // The thumbnail is smaller to load; the original replaces it if it cannot be shown.
  const shownHash = [thumbnail?.object_hash, asset.object_hash].find(
    (hash) => hash !== undefined && !failedHashes.has(hash),
  );
  if (shownHash === undefined) {
    return <p className="asset-original unavailable">Preview unavailable</p>;
  }
  return (
    <img
      key={shownHash}
      className="asset-original"
      src={objectUrl(shownHash)}
      alt={description}
      loading="lazy"
      onError={() => setFailedHashes((failed) => new Set(failed).add(shownHash))}
    />
  );
}

/** The Library thumbnail rendered from an original, if one is. */
export function thumbnailOf(asset: LibraryAsset) {
  return asset.derived.find(
    (derived) =>
      derived.recipe.transform === "thumbnail" &&
      derived.recipe.max_edge === LIBRARY_THUMBNAIL_EDGE,
  );
}

const COVERAGE_STATUS_LABELS: Record<CoverageStatus, string> = {
  partial: "Partial",
  packaging_complete: "Packaging Complete",
  physical_complete: "Physical Complete",
  archival_complete: "Archival Complete",
};

const PACKAGING_FAMILY_LABELS: Record<PackagingFamily, string> = {
  cardboard_box: "Cardboard box",
  jewel_case: "Jewel case",
  keep_case: "Keep case",
  cartridge_case: "Cartridge case",
  arcade_board: "Arcade board",
  digital_only: "Digital only",
};

const COVERAGE_PROFILE_LABELS: Record<CoverageProfile, string> = {
  packaging: "Packaging",
  physical: "Physical",
  archival: "Archival",
};

/** The Coverage Status of a release and what each of its profiles still misses. */
function ReleaseCoverageNote({ coverage }: { coverage: ReleaseCoverage | null }) {
  if (coverage === null) {
    return <p className="coverage">Coverage not evaluated for this platform</p>;
  }
  return (
    <div className="coverage">
      <strong>
        {COVERAGE_STATUS_LABELS[coverage.status]} ·{" "}
        {PACKAGING_FAMILY_LABELS[coverage.packaging_family]}
      </strong>
      {coverage.profiles
        .filter((profile) => profile.missing.length > 0)
        .map((profile) => (
          <span className="coverage-missing" key={profile.profile}>
            {COVERAGE_PROFILE_LABELS[profile.profile]} misses{" "}
            {profile.missing.map(assetTypeLabel).join(", ")}
          </span>
        ))}
    </div>
  );
}

/** Packaging families a 3D template builds, as SPEC §17 lists them. */
const TEMPLATED_FAMILIES: PackagingFamily[] = ["cardboard_box"];

/** The 3D box of a release, or what it still needs for one. */
function PackagingModelNote({
  entry,
  objectUrl,
}: {
  entry: LibraryEntry;
  objectUrl: (objectHash: string) => string;
}) {
  if (entry.packaging_model !== null) {
    return (
      <PackagingModelPreview
        url={objectUrl(entry.packaging_model.object_hash)}
        label={"3D box of " + entry.game_title}
      />
    );
  }
  const coverage = entry.coverage;
  if (coverage === null) {
    return null;
  }
  if (!TEMPLATED_FAMILIES.includes(coverage.packaging_family)) {
    return (
      <p className="packaging-model-note">
        No 3D template for {PACKAGING_FAMILY_LABELS[coverage.packaging_family].toLowerCase()}{" "}
        packaging yet
      </p>
    );
  }
  const missing =
    coverage.profiles.find((profile) => profile.profile === "packaging")?.missing ?? [];
  return (
    <p className="packaging-model-note">
      {missing.length > 0
        ? "3D box needs " + missing.map(assetTypeLabel).join(", ")
        : "3D box not built yet"}
    </p>
  );
}

/** Whether an original is preferred among the others of its type, and why if it is not. */
function AssetPreferenceNote({ entry, asset }: { entry: LibraryEntry; asset: LibraryAsset }) {
  const preferred = entry.preferred_assets.find(
    (candidate) => candidate.asset_type === asset.asset_type,
  );
  // A lone original of its type is not compared with anything.
  if (!preferred || preferred.outranks.length === 0) {
    return null;
  }
  if (preferred.asset_id === asset.asset_id) {
    return (
      <div className="asset-preference preferred">
        <span className="detail-label">Preference</span>
        <strong>Preferred</strong>
      </div>
    );
  }
  const outranked = preferred.outranks.find((other) => other.asset_id === asset.asset_id);
  if (!outranked) {
    return null;
  }
  return (
    <div className="asset-preference">
      <span className="detail-label">Preference</span>
      <span>{preferenceReason(outranked.reason)}</span>
    </div>
  );
}

function preferenceReason(reason: PreferenceReason) {
  switch (reason.reason) {
    case "more_pixels":
      return reason.other === null
        ? `Unknown pixel size, the preferred original has ${reason.preferred} px`
        : `Fewer pixels than the preferred original (${reason.other} vs ${reason.preferred} px)`;
    case "more_bytes":
      return `Same pixels in fewer bytes than the preferred original (${formatBytes(reason.other)} vs ${formatBytes(reason.preferred)})`;
    case "acquired_first":
      return "Equal to the preferred original, which was acquired first";
  }
}

/** Asset Types in taxonomy order, as the Download view lists them. */
const ASSET_TYPE_ORDER = ASSET_TYPE_FAMILIES.flatMap((family) => family.types).map(
  (type) => type.value,
);

/** The originals of a release grouped by Asset Type, in taxonomy order. */
export function byAssetType(assets: LibraryAsset[]) {
  const rank = (asset: LibraryAsset) => {
    const index = ASSET_TYPE_ORDER.indexOf(asset.asset_type);
    return index < 0 ? ASSET_TYPE_ORDER.length : index;
  };
  return [...assets].sort((a, b) => rank(a) - rank(b));
}

export function formatBytes(byteLength: number) {
  if (byteLength < 1024) {
    return String(byteLength) + " B";
  }
  if (byteLength < 1024 * 1024) {
    return (byteLength / 1024).toFixed(1) + " KiB";
  }
  return (byteLength / (1024 * 1024)).toFixed(1) + " MiB";
}

/** Video containers the webview plays itself. */
const PLAYABLE_VIDEO_TYPES = ["video/mp4", "video/webm"];

/**
 * Whether an original is an image. Assets imported before media inspection (vaults upgraded
 * from schema version 2) still have unknown media, so their file name decides until the vault
 * is verified again.
 */
export function isImageOriginal(asset: LibraryAsset) {
  const mediaType =
    asset.media_type === "application/octet-stream"
      ? filenameMediaType(asset.original_filename)
      : asset.media_type;
  return mediaType.startsWith("image/");
}

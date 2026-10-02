import { FormEvent, useState } from "react";

import { ASSET_TYPE_FAMILIES, assetTypeLabel } from "./acquisition";
import {
  filenameMediaType,
  type CoverageProfile,
  type CoverageStatus,
  type LibraryAsset,
  type LibraryEntry,
  type LibraryFilters,
  type LibraryStatus,
  NO_LIBRARY_FILTERS,
  narrowsLibrary,
  type PackagingFamily,
  type PreferenceReason,
  type ReleaseAssertionField,
  type ReleaseCoverage,
} from "./types";

interface LibraryViewProps {
  entries: LibraryEntry[];
  /** URL under which the desktop shell serves the original object with this hash. */
  objectUrl: (objectHash: string) => string;
  /** Filters of the current search; the filter bar shows only with `onSearch`. */
  filters?: LibraryFilters;
  /** Changes whenever a search settles, resetting the filter bar to `filters`. */
  filtersRevision?: number;
  onSearch?: (filters: LibraryFilters) => void;
  canLoadMore?: boolean;
  /** Whether the next page is loading, which disables asking for it again. */
  loadingMore?: boolean;
  /** Whether a filter search is pending, which blocks paging the previous results. */
  searching?: boolean;
  onLoadMore?: () => void;
}

const STATUS_OPTIONS: [LibraryStatus, string][] = [
  ["complete", "Complete"],
  ["partial", "Partial"],
  ["needs_review", "Needs review"],
];

export function LibraryView({
  entries,
  objectUrl,
  filters = NO_LIBRARY_FILTERS,
  filtersRevision = 0,
  onSearch,
  canLoadMore = false,
  loadingMore = false,
  searching = false,
  onLoadMore,
}: LibraryViewProps) {
  return (
    <>
      {onSearch ? (
        <LibraryFilterBar key={filtersRevision} filters={filters} onSearch={onSearch} />
      ) : null}
      <LibraryResults
        entries={entries}
        objectUrl={objectUrl}
        filtered={narrowsLibrary(filters)}
      />
      {canLoadMore && onLoadMore ? (
        <button
          type="button"
          className="load-more"
          disabled={loadingMore || searching}
          onClick={onLoadMore}
        >
          {loadingMore ? "Loading more…" : "Load more"}
        </button>
      ) : null}
    </>
  );
}

/** One value per line, without blank lines or surrounding spaces. */
function lines(text: string) {
  return text
    .split(/\r?\n/)
    .map((value) => value.trim())
    .filter((value) => value.length > 0);
}

/** Library filters, applied together when the search is submitted. */
function LibraryFilterBar({
  filters,
  onSearch,
}: {
  filters: LibraryFilters;
  onSearch: (filters: LibraryFilters) => void;
}) {
  const [text, setText] = useState(filters.text);
  const [platforms, setPlatforms] = useState(filters.platforms.join("\n"));
  const [regions, setRegions] = useState(filters.regions.join("\n"));
  const [sources, setSources] = useState(filters.sources.join("\n"));
  const [assetTypes, setAssetTypes] = useState(filters.assetTypes);
  const [statuses, setStatuses] = useState(filters.statuses);

  function toggle(status: LibraryStatus) {
    setStatuses((current) =>
      current.includes(status) ? current.filter((value) => value !== status) : [...current, status],
    );
  }

  return (
    <form
      className="library-filters"
      aria-label="Library filters"
      onSubmit={(event: FormEvent) => {
        event.preventDefault();
        onSearch({
          text,
          platforms: lines(platforms),
          regions: lines(regions),
          sources: lines(sources),
          assetTypes,
          statuses,
        });
      }}
    >
      <label>
        Search titles
        <input value={text} onChange={(event) => setText(event.target.value)} />
      </label>
      <label>
        Platforms (one per line)
        <textarea rows={2} value={platforms} onChange={(event) => setPlatforms(event.target.value)} />
      </label>
      <label>
        Regions (one per line)
        <textarea rows={2} value={regions} onChange={(event) => setRegions(event.target.value)} />
      </label>
      <label>
        Sources (one per line)
        <textarea rows={2} value={sources} onChange={(event) => setSources(event.target.value)} />
      </label>
      <label>
        Asset Types
        <select
          multiple
          value={assetTypes}
          onChange={(event) =>
            setAssetTypes(Array.from(event.target.selectedOptions, (option) => option.value))
          }
        >
          {ASSET_TYPE_FAMILIES.map((family) => (
            <optgroup label={family.label} key={family.value}>
              <option value={family.value}>All {family.label}</option>
              {family.types.map((type) => (
                <option value={type.value} key={type.value}>
                  {type.label}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
      </label>
      {STATUS_OPTIONS.map(([status, label]) => (
        <label className="choice" key={status}>
          <input
            type="checkbox"
            checked={statuses.includes(status)}
            onChange={() => toggle(status)}
          />
          {label}
        </label>
      ))}
      <button type="submit">Search</button>
    </form>
  );
}

interface LibraryResultsProps {
  entries: LibraryEntry[];
  objectUrl: (objectHash: string) => string;
  /** Whether filters narrowed the search, which explains an empty result. */
  filtered: boolean;
}

function LibraryResults({ entries, objectUrl, filtered }: LibraryResultsProps) {
  if (entries.length === 0) {
    return filtered ? (
      <section className="empty-state" aria-live="polite">
        <h2>No releases match these filters</h2>
        <p>Change or clear the filters to see more of the Library.</p>
      </section>
    ) : (
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
            <ReleaseCoverageNote coverage={entry.coverage} />
          </div>

          <div className="release-assets">
            {entry.assets.length === 0 ? (
              <p className="no-assets">No media assets</p>
            ) : (
              byAssetType(entry.assets).map((asset) => (
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
                    <AssetPreferenceNote entry={entry} asset={asset} />
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

/** Asset Types in taxonomy order, as the Acquire view lists them. */
const ASSET_TYPE_ORDER = ASSET_TYPE_FAMILIES.flatMap((family) => family.types).map(
  (type) => type.value,
);

/** The originals of a release grouped by Asset Type, in taxonomy order. */
function byAssetType(assets: LibraryAsset[]) {
  const rank = (asset: LibraryAsset) => {
    const index = ASSET_TYPE_ORDER.indexOf(asset.asset_type);
    return index < 0 ? ASSET_TYPE_ORDER.length : index;
  };
  return [...assets].sort((a, b) => rank(a) - rank(b));
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

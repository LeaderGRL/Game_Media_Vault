import { type FormEvent, useState } from "react";

import { ASSET_TYPE_FAMILIES, KNOWN_SOURCES, assetTypeLabel } from "./acquisition";
import { REGIONS, consoleName } from "./catalog";
import { Dialog, FilterMenu } from "./controls";
import { ExportPanel } from "./ExportPanel";
import { Icon } from "./icons";
import { FallbackImage, ReleaseDetail, coverOf, thumbnailOf } from "./ReleaseDetail";
import {
  type ExportSummary,
  type LibraryEntry,
  type LibraryFilters,
  type LibraryStatus,
  NO_LIBRARY_FILTERS,
  narrowsLibrary,
} from "./types";

export { LIBRARY_THUMBNAIL_EDGE } from "./ReleaseDetail";

interface LibraryViewProps {
  entries: LibraryEntry[];
  /** Games matching the filters across every page. */
  total: number;
  /** URL under which the desktop shell serves the original object with this hash. */
  objectUrl: (objectHash: string) => string;
  /** Filters of the shown results. */
  filters?: LibraryFilters;
  /** Changes whenever a search settles, resetting the search field to `filters`. */
  filtersRevision?: number;
  /** Every platform whose games have media, for the console filter. */
  platforms?: string[];
  onSearch?: (filters: LibraryFilters) => void;
  canLoadMore?: boolean;
  /** Whether the next page is loading, which disables asking for it again. */
  loadingMore?: boolean;
  /** Whether a filter search is pending, which blocks paging the previous results. */
  searching?: boolean;
  onLoadMore?: () => void;
  /** Copies the vault's media to a folder; the Export button shows only with it. */
  onExport?: (destination: string) => Promise<ExportSummary>;
  /** Opens the Download view, offered when the library is empty. */
  onDownload?: () => void;
  /** Whether media arrived since the shown results were read. */
  newMedia?: boolean;
  onRefresh?: () => void;
}

const STATUS_OPTIONS: { value: LibraryStatus; label: string }[] = [
  { value: "complete", label: "Complete" },
  { value: "partial", label: "Partial" },
  { value: "needs_review", label: "Needs review" },
];

/** The Sources a release's media or records can come from: media Sources, reference catalogs
 * and local imports. */
const SOURCE_OPTIONS = [
  ...KNOWN_SOURCES,
  { value: "no-intro", label: "No-Intro" },
  { value: "redump", label: "Redump" },
  { value: "mame-software-lists", label: "MAME software lists" },
  { value: "local_import", label: "Local import" },
];

/** Each media family, which also matches the types added to it later, then its own types. */
const MEDIA_OPTIONS = ASSET_TYPE_FAMILIES.flatMap((family) => [
  { value: family.value, label: `All ${family.label}` },
  ...family.types.map((type) => ({ value: type.value, label: type.label })),
]);

/** The games of the vault as a grid of covers, filtered by lists, each opening its media. */
export function LibraryView({
  entries,
  total,
  objectUrl,
  filters = NO_LIBRARY_FILTERS,
  filtersRevision = 0,
  platforms = [],
  onSearch,
  canLoadMore = false,
  loadingMore = false,
  searching = false,
  onLoadMore,
  onExport,
  onDownload,
  newMedia = false,
  onRefresh,
}: LibraryViewProps) {
  // The filters being searched, which become `filters` again whenever a search settles; the
  // reset happens while rendering, so the bar never shows them a moment after the results.
  const [draft, setDraft] = useState(filters);
  const [shown, setShown] = useState({ filters, filtersRevision });
  if (shown.filters !== filters || shown.filtersRevision !== filtersRevision) {
    setShown({ filters, filtersRevision });
    setDraft(filters);
  }
  const [openedId, setOpenedId] = useState<number | null>(null);
  const [exporting, setExporting] = useState(false);
  const opened = entries.find((entry) => entry.release_edition_id === openedId);

  function search(change: Partial<LibraryFilters>) {
    const next = { ...draft, ...change };
    setDraft(next);
    onSearch?.(next);
  }

  return (
    <>
      <div className="library-toolbar">
        <form
          className="search-field"
          role="search"
          onSubmit={(event: FormEvent) => {
            event.preventDefault();
            search({});
          }}
        >
          <Icon name="search" size={18} />
          <input
            aria-label="Search titles"
            placeholder="Search titles"
            value={draft.text}
            onChange={(event) => setDraft({ ...draft, text: event.target.value })}
          />
        </form>
        <FilterMenu
          label="Consoles"
          options={platforms.map((platform) => ({ value: platform, label: consoleName(platform) }))}
          selected={draft.platforms}
          onChange={(selected) => search({ platforms: selected })}
        />
        <FilterMenu
          label="Regions"
          options={REGIONS}
          selected={draft.regions}
          onChange={(selected) => search({ regions: selected })}
        />
        <FilterMenu
          label="Source"
          options={SOURCE_OPTIONS}
          selected={draft.sources}
          onChange={(selected) => search({ sources: selected })}
        />
        <FilterMenu
          label="Media"
          options={MEDIA_OPTIONS}
          selected={draft.assetTypes}
          onChange={(selected) => search({ assetTypes: selected })}
        />
        <FilterMenu
          label="Status"
          options={STATUS_OPTIONS}
          selected={draft.statuses}
          onChange={(selected) => search({ statuses: selected as LibraryStatus[] })}
        />
        <label className="choice">
          <input
            type="checkbox"
            checked={draft.includeWithoutMedia}
            onChange={(event) => search({ includeWithoutMedia: event.target.checked })}
          />
          Include games without media
        </label>
        {onExport ? (
          <button type="button" onClick={() => setExporting(true)}>
            <Icon name="export" size={18} />
            Export…
          </button>
        ) : null}
      </div>

      {newMedia && onRefresh ? (
        <div className="banner" role="status">
          <Icon name="sparkle" size={18} />
          New media arrived.
          <button type="button" onClick={onRefresh}>
            Show them
          </button>
        </div>
      ) : null}

      {entries.length === 0 ? (
        narrowsLibrary(filters) ? (
          <section className="empty-state" aria-live="polite">
            <span className="empty-icon">
              <Icon name="search" size={26} />
            </span>
            <h2>No games match these filters</h2>
            <p>Change or clear the filters to see more of your library.</p>
          </section>
        ) : (
          <section className="empty-state" aria-live="polite">
            <span className="empty-icon">
              <Icon name="library" size={26} />
            </span>
            <h2>Your library is empty</h2>
            <p>Choose consoles to download: covers, discs, manuals and more arrive here.</p>
            {onDownload ? (
              <button type="button" className="primary" onClick={onDownload}>
                <Icon name="download" size={18} />
                Download media
              </button>
            ) : null}
          </section>
        )
      ) : (
        <>
          <p className="library-meta">
            {total} {total === 1 ? "release" : "releases"}
          </p>
          <section className="cover-grid" aria-label="Library games">
            {entries.map((entry) => (
              <CoverCard
                key={entry.release_edition_id}
                entry={entry}
                objectUrl={objectUrl}
                onOpen={() => setOpenedId(entry.release_edition_id)}
              />
            ))}
          </section>
        </>
      )}

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

      {opened ? (
        <>
          <div className="drawer-backdrop" onMouseDown={() => setOpenedId(null)} />
          <aside className="drawer" role="dialog" aria-modal="true" aria-label={opened.game_title}>
            <ReleaseDetail
              entry={opened}
              objectUrl={objectUrl}
              onClose={() => setOpenedId(null)}
            />
          </aside>
        </>
      ) : null}

      {exporting && onExport ? (
        <Dialog title="Export media" onClose={() => setExporting(false)}>
          <p className="hint">
            Copies every original to a folder you can browse, as console / game / media type /
            file. Exporting again copies only what is new.
          </p>
          <ExportPanel onExport={onExport} />
        </Dialog>
      ) : null}
    </>
  );
}

/** A game as its cover, title, console and media count; it opens the game's media. */
function CoverCard({
  entry,
  objectUrl,
  onOpen,
}: {
  entry: LibraryEntry;
  objectUrl: (objectHash: string) => string;
  onOpen: () => void;
}) {
  const cover = coverOf(entry);
  const count = entry.assets.length;
  return (
    <button type="button" className="cover-card" onClick={onOpen}>
      <div className="cover-frame">
        <FallbackImage
          hashes={cover ? [thumbnailOf(cover)?.object_hash, cover.object_hash] : []}
          objectUrl={objectUrl}
          alt={cover ? `${assetTypeLabel(cover.asset_type)} of ${entry.game_title}` : ""}
          fallback={
            <div className="media-placeholder">
              <Icon name="gamepad" size={34} />
            </div>
          }
        />
        {count > 0 ? <span className="cover-badge">{count}</span> : null}
      </div>
      <span className="cover-title">{entry.game_title}</span>
      <span className="cover-meta">
        {consoleName(entry.platform)} · {entry.region}
      </span>
      <span className="cover-meta">
        {count} {count === 1 ? "medium" : "media"}
      </span>
    </button>
  );
}

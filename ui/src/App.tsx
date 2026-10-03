import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { FormEvent, useCallback, useEffect, useRef, useState } from "react";

import { AcquireView } from "./AcquireView";
import type {
  AcquisitionPlan,
  AcquisitionRequestDraft,
  AcquisitionRun,
  SourceDescription,
  SourceFailureSummary,
} from "./acquisition";
import { LIBRARY_THUMBNAIL_EDGE, LibraryView } from "./LibraryView";
import { ReviewView } from "./ReviewView";
import { RunsView } from "./RunsView";
import { SourcesView } from "./SourcesView";
import { ReferenceImportForm } from "./ReferenceImportForm";
import { ReferenceReviewView } from "./ReferenceReviewView";
import { NO_LIBRARY_FILTERS, errorMessage } from "./types";
import type {
  DerivationSummary,
  LibraryEntry,
  LibraryFilters,
  LibraryPage,
  PackagingModelSummary,
  ReferenceImportInput,
  ReferenceImportSummary,
  ReferenceReviewItem,
  ReviewDecision,
  ReviewItem,
} from "./types";

type View = "library" | "review" | "acquire" | "runs" | "sources";

type RunAction = "pause" | "resume" | "cancel";

/** Default thresholds used by desktop executions (SPEC §10 keeps them configurable). */
const MATCHING_POLICY = { high_confidence_threshold: 80, medium_confidence_threshold: 50 };

const EMPTY_LIBRARY_PAGE: LibraryPage = { releases: [], total: 0, next_after: null, as_of: 0 };

/** A long Library task the backend runs for one vault, such as rendering its thumbnails. */
interface VaultLibraryTask<Summary> {
  /**
   * Vaults running this task: the backend keeps running a vault's task while another vault is
   * loaded, so loading it again shows the task still running instead of offering another.
   */
  runningVaults: { current: Set<string> };
  setRunning: (running: boolean) => void;
  setStatus: (status: string | null) => void;
  run: () => Promise<Summary>;
  describe: (summary: Summary) => string;
}

/** Latest failures the Sources view shows for each Source. */
const SOURCE_FAILURES_SHOWN = 3;

/** How often run counts are refreshed while a run executes. */
export const RUN_PROGRESS_REFRESH_MS = 3000;

export function App() {
  const activeVaultRoot = useRef<string | null>(null);
  // The identity the backend resolved for each spelling of a vault path the user loaded.
  const vaultIdentities = useRef(new Map<string, string>());
  // The vault the backend has open, unset while another one opens; commands issued for any
  // other vault would read the wrong catalog.
  const openedVaultRoot = useRef<string | null>(null);
  const vaultLoadRequestGeneration = useRef(0);
  const reviewMutationGeneration = useRef(0);
  const reviewRefreshRequestGeneration = useRef(0);
  // Each refresh of these lists takes a new generation; only the newest one is applied.
  const runListGeneration = useRef(0);
  // Progress polls of the executing runs are ordered among themselves and give way to any full
  // run list refresh started after them, without discarding it.
  const runPollGeneration = useRef(0);
  const vaultDataGeneration = useRef(0);
  const activeViewRef = useRef<View>("library");
  // Executions keep running in the backend while another vault is loaded, so loading their
  // vault again shows them executing instead of offering to start them a second time.
  const executionsByVault = useRef(new Map<string | null, Set<number>>());
  const [vaultRoot, setVaultRoot] = useState(".game-media-vault");
  const [loadedVaultRoot, setLoadedVaultRoot] = useState<string | null>(null);
  const [entries, setEntries] = useState<LibraryEntry[]>([]);
  // Searches use the filters of the latest search, including those that refresh the Library.
  const libraryFiltersRef = useRef<LibraryFilters>(NO_LIBRARY_FILTERS);
  const [libraryFilters, setLibraryFilters] = useState<LibraryFilters>(NO_LIBRARY_FILTERS);
  // Counts settled filter searches; the filter bar then shows the filters of the shown results.
  const [libraryFiltersRevision, setLibraryFiltersRevision] = useState(0);
  const [libraryTotal, setLibraryTotal] = useState(0);
  const [libraryNextAfter, setLibraryNextAfter] = useState<number | null>(null);
  // The newest Release Edition the first page searched; later pages keep to its results.
  const [libraryAsOf, setLibraryAsOf] = useState<number | null>(null);
  // The pending next-page request, if any: only it may apply its page or end the loading state.
  const pageRequestRef = useRef<object | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  // Filters of the pending filter search; paging would continue the previous filters meanwhile.
  const pendingSearchRef = useRef<LibraryFilters | null>(null);
  const [searchingLibrary, setSearchingLibrary] = useState(false);
  const [reviewItems, setReviewItems] = useState<ReviewItem[]>([]);
  // The reference records awaiting review in the loaded vault, and those being decided.
  const [referenceReviewItems, setReferenceReviewItems] = useState<ReferenceReviewItem[]>([]);
  const [decidingReferenceIds, setDecidingReferenceIds] = useState<Set<number>>(() => new Set());
  // The latest read or decision of the reference records: an older one settling later applies
  // nothing.
  const referenceReviewGeneration = useRef(0);
  const [activeView, setActiveView] = useState<View>("library");
  const [loading, setLoading] = useState(false);
  const [resolvingIds, setResolvingIds] = useState<Set<number>>(() => new Set());
  const [runs, setRuns] = useState<AcquisitionRun[]>([]);
  const [busyRunIds, setBusyRunIds] = useState<Set<number>>(() => new Set());
  const [executingRunIds, setExecutingRunIds] = useState<Set<number>>(() => new Set());
  const [startingRun, setStartingRun] = useState(false);
  // Vaults whose thumbnails are rendering: the backend keeps rendering a vault while another is
  // loaded, so loading it again shows its rendering instead of offering to start another.
  const renderingThumbnailVaults = useRef(new Set<string>());
  const [renderingThumbnails, setRenderingThumbnails] = useState(false);
  const [thumbnailStatus, setThumbnailStatus] = useState<string | null>(null);
  // Vaults whose packaging models are building, kept like thumbnail renderings.
  const buildingModelVaults = useRef(new Set<string>());
  const [buildingModels, setBuildingModels] = useState(false);
  const [modelStatus, setModelStatus] = useState<string | null>(null);
  // Vaults whose reference import runs: the backend keeps importing a vault while another is
  // loaded, so loading it again shows its import instead of offering to start another.
  const importingReferenceVaults = useRef(new Set<string>());
  const [importingReference, setImportingReference] = useState(false);
  const [referenceImportStatus, setReferenceImportStatus] = useState<string | null>(null);
  // The registered Sources, read the first time the Sources view is shown; they need no vault.
  const [sources, setSources] = useState<SourceDescription[] | null>(null);
  // Why the Sources could not be read the last time the Sources view was shown.
  const [sourcesError, setSourcesError] = useState<string | null>(null);
  // The latest read of the Sources: an older one settling later applies nothing.
  const sourcesRequest = useRef(0);
  // The failures the loaded vault recorded by Source, read each time the Sources view is shown.
  const [sourceFailures, setSourceFailures] = useState<SourceFailureSummary[] | null>(null);
  const sourceFailuresRequest = useRef(0);
  const [error, setError] = useState<string | null>(null);
  const releaseCountLabel = `${libraryTotal} ${libraryTotal === 1 ? "release" : "releases"}`;
  const reviewCount = reviewItems.length + referenceReviewItems.length;
  const reviewCountLabel = `${reviewCount} ${reviewCount === 1 ? "review" : "reviews"}`;

  /**
   * Searches a page of the Library with `filters`: the first one, or the one after `after`
   * among the results of the first page, whose `as_of` it passes back. The backend chooses the
   * page size.
   */
  function searchLibrary(
    filters: LibraryFilters = libraryFiltersRef.current,
    after: number | null = null,
    asOf: number | null = null,
  ) {
    const text = filters.text.trim();
    return invoke<LibraryPage>("search_library", {
      query: {
        text: text === "" ? null : text,
        platforms: filters.platforms,
        regions: filters.regions,
        sources: filters.sources,
        asset_types: filters.assetTypes,
        statuses: filters.statuses,
        after,
        as_of: asOf,
      },
    });
  }

  /** Shows a first page; a next page requested before it would extend other results. */
  function showLibraryPage(page: LibraryPage) {
    abandonPageRequest();
    setEntries(page.releases);
    setLibraryTotal(page.total);
    setLibraryNextAfter(page.next_after);
    setLibraryAsOf(page.as_of);
  }

  /**
   * Searches with new filters, which apply only together with their first page: a failed
   * search keeps the previous filters with the results they produced. Once the search settles,
   * the filter bar shows the filters of the shown results.
   */
  async function applyLibraryFilters(filters: LibraryFilters) {
    if (loadedVaultRoot === null || openedVaultRoot.current !== loadedVaultRoot) {
      return;
    }
    const searchingVaultRoot = loadedVaultRoot;
    // Supersedes refreshes and pages requested with the previous filters, and a pending search
    // whose filters the bar keeps showing until this one settles.
    pendingSearchRef.current = null;
    const generation = supersedeLibraryRequests();
    pendingSearchRef.current = filters;
    setSearchingLibrary(true);
    setError(null);
    try {
      const page = await searchLibrary(filters);
      if (
        activeVaultRoot.current === searchingVaultRoot &&
        generation === vaultDataGeneration.current
      ) {
        libraryFiltersRef.current = filters;
        setLibraryFilters(filters);
        showLibraryPage(page);
        setLibraryFiltersRevision((revision) => revision + 1);
      }
    } catch (reason) {
      // A newer search or refresh reports for itself.
      if (
        activeVaultRoot.current === searchingVaultRoot &&
        generation === vaultDataGeneration.current
      ) {
        setError(errorMessage(reason));
        setLibraryFiltersRevision((revision) => revision + 1);
      }
    } finally {
      if (generation === vaultDataGeneration.current) {
        pendingSearchRef.current = null;
        setSearchingLibrary(false);
      }
    }
  }

  /**
   * Starts a request that replaces the shown Library page: searches, refreshes and pages begun
   * earlier no longer apply. A superseded filter search never applies its filters, so the
   * filter bar shows the applied ones again.
   */
  function supersedeLibraryRequests() {
    vaultDataGeneration.current += 1;
    if (pendingSearchRef.current !== null) {
      pendingSearchRef.current = null;
      setSearchingLibrary(false);
      setLibraryFiltersRevision((revision) => revision + 1);
    }
    abandonPageRequest();
    return vaultDataGeneration.current;
  }

  /** Forgets the pending next page, so the results that replace it can be paged at once. */
  function abandonPageRequest() {
    pageRequestRef.current = null;
    setLoadingMore(false);
  }

  /**
   * Appends the next page, one request at a time. It applies only to the results it extends:
   * a search, refresh or vault load started or shown meanwhile abandons it.
   */
  async function loadMoreReleases() {
    if (
      libraryNextAfter === null ||
      pageRequestRef.current !== null ||
      searchingLibrary ||
      openedVaultRoot.current !== loadedVaultRoot
    ) {
      return;
    }
    const request = {};
    pageRequestRef.current = request;
    setLoadingMore(true);
    setError(null);
    try {
      const page = await searchLibrary(libraryFiltersRef.current, libraryNextAfter, libraryAsOf);
      if (pageRequestRef.current === request) {
        setEntries((current) => [...current, ...page.releases]);
        setLibraryTotal(page.total);
        setLibraryNextAfter(page.next_after);
      }
    } catch (reason) {
      // An abandoned page leaves reporting to whatever replaced its results.
      if (pageRequestRef.current === request) {
        setError(errorMessage(reason));
      }
    } finally {
      if (pageRequestRef.current === request) {
        abandonPageRequest();
      }
    }
  }

  async function loadVault(create: boolean) {
    const requestedVaultRoot = vaultRoot;
    // Vault-scoped state is keyed by the identity the backend resolves for the vault, shared by
    // every spelling of its path; the identity this spelling resolved to before stands in until
    // the backend answers.
    let vaultKey = vaultIdentities.current.get(requestedVaultRoot) ?? requestedVaultRoot;
    activeVaultRoot.current = vaultKey;
    openedVaultRoot.current = null;
    vaultLoadRequestGeneration.current += 1;
    const loadGeneration = vaultLoadRequestGeneration.current;
    const reviewGenerationAtLoadStart = reviewMutationGeneration.current;
    reviewRefreshRequestGeneration.current += 1;
    setLoading(true);
    setError(null);
    supersedeLibraryRequests();
    showLibraryPage(EMPTY_LIBRARY_PAGE);
    setThumbnailStatus(null);
    setRenderingThumbnails(renderingThumbnailVaults.current.has(vaultKey));
    setModelStatus(null);
    setBuildingModels(buildingModelVaults.current.has(vaultKey));
    setImportingReference(importingReferenceVaults.current.has(vaultKey));
    setReferenceImportStatus(null);
    // Another vault's failures never show, even from a read that settles later.
    sourceFailuresRequest.current += 1;
    setSourceFailures(null);
    setReviewItems([]);
    referenceReviewGeneration.current += 1;
    setReferenceReviewItems([]);
    setDecidingReferenceIds(new Set());
    setLoadedVaultRoot(null);
    setResolvingIds(new Set());
    setRuns([]);
    setBusyRunIds(new Set());
    setExecutingRunIds(new Set(executionsByVault.current.get(vaultKey)));
    try {
      // The backend keeps the opened vault; later commands never send a path.
      const identity = await invoke<string>("open_vault", {
        vault_root: requestedVaultRoot,
        create,
      });
      if (loadGeneration !== vaultLoadRequestGeneration.current) {
        return;
      }
      vaultIdentities.current.set(requestedVaultRoot, identity);
      if (identity !== vaultKey) {
        vaultKey = identity;
        activeVaultRoot.current = identity;
        setRenderingThumbnails(renderingThumbnailVaults.current.has(identity));
        setBuildingModels(buildingModelVaults.current.has(identity));
        setImportingReference(importingReferenceVaults.current.has(identity));
        setExecutingRunIds(new Set(executionsByVault.current.get(identity)));
      }
      openedVaultRoot.current = vaultKey;
      // A refresh started after this search, such as one after a rendering, shows newer data.
      const libraryGeneration = supersedeLibraryRequests();
      const [library, reviews] = await Promise.all([
        searchLibrary(),
        invoke<ReviewItem[]>("list_review_items"),
        refreshReferenceReviews(vaultKey),
      ]);
      if (
        activeVaultRoot.current !== vaultKey ||
        loadGeneration !== vaultLoadRequestGeneration.current
      ) {
        return;
      }
      // A decision made meanwhile refreshes both lists itself; this load read them before it.
      if (reviewMutationGeneration.current === reviewGenerationAtLoadStart) {
        if (libraryGeneration === vaultDataGeneration.current) {
          showLibraryPage(library);
        }
        setReviewItems(reviews);
      }
      setLoadedVaultRoot(vaultKey);
      if (activeViewRef.current === "runs") {
        await refreshRuns(vaultKey);
      }
    } catch (reason) {
      if (
        activeVaultRoot.current === vaultKey &&
        loadGeneration === vaultLoadRequestGeneration.current
      ) {
        setError(errorMessage(reason));
      }
    } finally {
      if (
        activeVaultRoot.current === vaultKey &&
        loadGeneration === vaultLoadRequestGeneration.current
      ) {
        setLoading(false);
      }
    }
  }

  async function resolveReviewItem(reviewItemId: number, decision: ReviewDecision) {
    if (loadedVaultRoot === null) {
      setError("Load a vault before resolving review items.");
      return;
    }
    setResolvingIds((current) => {
      const next = new Set(current);
      next.add(reviewItemId);
      return next;
    });
    setError(null);
    const resolvingVaultRoot = loadedVaultRoot;
    try {
      const resolvedReviewItem = await invoke<ReviewItem>("resolve_review_item", {
        review_item_id: reviewItemId,
        decision,
      });
      if (activeVaultRoot.current !== resolvingVaultRoot) {
        return;
      }
      reviewMutationGeneration.current += 1;
      setReviewItems((current) => {
        const existingIndex = current.findIndex((item) => item.id === reviewItemId);
        if (existingIndex < 0) {
          return [...current, resolvedReviewItem];
        }
        return current.map((item) => (item.id === reviewItemId ? resolvedReviewItem : item));
      });
      reviewRefreshRequestGeneration.current += 1;
      const resolvingRefreshGeneration = reviewRefreshRequestGeneration.current;
      const libraryGeneration = supersedeLibraryRequests();
      // Decisions can attach or detach the candidate's asset, so the library is refreshed too.
      const [reviews, library] = await Promise.all([
        invoke<ReviewItem[]>("list_review_items"),
        searchLibrary(),
        // Accepting requeues parked work and may reopen completed runs.
        refreshRuns(resolvingVaultRoot),
      ]);
      if (
        activeVaultRoot.current !== resolvingVaultRoot ||
        resolvingRefreshGeneration !== reviewRefreshRequestGeneration.current
      ) {
        return;
      }
      setReviewItems(reviews);
      if (libraryGeneration === vaultDataGeneration.current) {
        showLibraryPage(library);
      }
    } catch (reason) {
      if (activeVaultRoot.current === resolvingVaultRoot) {
        setError(errorMessage(reason));
        // A refused decision usually means the item changed elsewhere, possibly linking or
        // detaching its asset; show the current reviews and library.
        reviewRefreshRequestGeneration.current += 1;
        const refusalRefreshGeneration = reviewRefreshRequestGeneration.current;
        const refusalLibraryGeneration = supersedeLibraryRequests();
        try {
          const [reviews, library] = await Promise.all([
            invoke<ReviewItem[]>("list_review_items"),
            searchLibrary(),
          ]);
          if (
            activeVaultRoot.current === resolvingVaultRoot &&
            refusalRefreshGeneration === reviewRefreshRequestGeneration.current
          ) {
            setReviewItems(reviews);
            if (refusalLibraryGeneration === vaultDataGeneration.current) {
              showLibraryPage(library);
            }
          }
        } catch {
          // The refused decision stays the reported error.
        }
      }
    } finally {
      if (activeVaultRoot.current === resolvingVaultRoot) {
        setResolvingIds((current) => {
          const next = new Set(current);
          next.delete(reviewItemId);
          return next;
        });
      }
    }
  }

  /** Reads the reference records awaiting review in the opened vault `expectedVaultRoot`. */
  async function refreshReferenceReviews(expectedVaultRoot: string) {
    if (openedVaultRoot.current !== expectedVaultRoot) {
      return;
    }
    referenceReviewGeneration.current += 1;
    const generation = referenceReviewGeneration.current;
    const listed = await invoke<ReferenceReviewItem[]>("list_reference_review_items");
    if (
      activeVaultRoot.current === expectedVaultRoot &&
      generation === referenceReviewGeneration.current
    ) {
      setReferenceReviewItems(listed);
    }
  }

  /**
   * Links a reference record to one of its candidates or keeps it apart through `decide`, which
   * answers the records still pending, then shows the Library the decision changed.
   */
  async function decideReferenceReview(
    itemId: number,
    decide: () => Promise<ReferenceReviewItem[]>,
  ) {
    if (loadedVaultRoot === null) {
      return;
    }
    const decidingVaultRoot = loadedVaultRoot;
    setDecidingReferenceIds((current) => new Set(current).add(itemId));
    setError(null);
    try {
      const pending = await decide();
      if (activeVaultRoot.current !== decidingVaultRoot) {
        return;
      }
      referenceReviewGeneration.current += 1;
      setReferenceReviewItems(pending);
      // Linking merges or moves records between editions, which the Library shows.
      await showChangedLibrary(decidingVaultRoot);
    } catch (reason) {
      if (activeVaultRoot.current === decidingVaultRoot) {
        setError(errorMessage(reason));
        // A refused decision usually means the record changed meanwhile.
        try {
          await refreshReferenceReviews(decidingVaultRoot);
        } catch {
          // The refused decision stays the reported error.
        }
      }
    } finally {
      if (activeVaultRoot.current === decidingVaultRoot) {
        setDecidingReferenceIds((current) => {
          const next = new Set(current);
          next.delete(itemId);
          return next;
        });
      }
    }
  }

  function showView(view: View) {
    activeViewRef.current = view;
    setActiveView(view);
  }

  async function refreshRuns(expectedVaultRoot: string | null) {
    if (openedVaultRoot.current !== expectedVaultRoot) {
      return;
    }
    runListGeneration.current += 1;
    const generation = runListGeneration.current;
    const listed = await invoke<AcquisitionRun[]>("list_acquisition_runs");
    if (activeVaultRoot.current === expectedVaultRoot && generation === runListGeneration.current) {
      setRuns(listed);
    }
  }

  /** Reloads only the executing runs, so progress polling does not list the whole history. */
  async function refreshExecutingRuns(runIds: number[], expectedVaultRoot: string | null) {
    if (openedVaultRoot.current !== expectedVaultRoot) {
      return;
    }
    const listGeneration = runListGeneration.current;
    runPollGeneration.current += 1;
    const pollGeneration = runPollGeneration.current;
    const loaded = await Promise.all(
      runIds.map((runId) => invoke<AcquisitionRun>("get_acquisition_run", { run_id: runId })),
    );
    if (
      activeVaultRoot.current === expectedVaultRoot &&
      listGeneration === runListGeneration.current &&
      pollGeneration === runPollGeneration.current
    ) {
      setRuns((current) =>
        current.map((run) => loaded.find((loadedRun) => loadedRun.id === run.id) ?? run),
      );
    }
  }

  async function refreshVaultData(expectedVaultRoot: string | null) {
    if (openedVaultRoot.current !== expectedVaultRoot) {
      return;
    }
    const generation = supersedeLibraryRequests();
    reviewRefreshRequestGeneration.current += 1;
    const reviewRefreshGeneration = reviewRefreshRequestGeneration.current;
    const reviewGenerationAtStart = reviewMutationGeneration.current;
    const [library, reviews] = await Promise.all([
      searchLibrary(),
      invoke<ReviewItem[]>("list_review_items"),
    ]);
    // A review decision made meanwhile read both lists after this one.
    if (
      activeVaultRoot.current !== expectedVaultRoot ||
      reviewMutationGeneration.current !== reviewGenerationAtStart
    ) {
      return;
    }
    // A filter search replaces only the Library page; the Review Items stay this refresh's
    // unless a newer refresh read them.
    if (generation === vaultDataGeneration.current) {
      showLibraryPage(library);
    }
    if (reviewRefreshGeneration === reviewRefreshRequestGeneration.current) {
      setReviewItems(reviews);
    }
  }

  /**
   * Runs a long Library task of the opened vault, one at a time per vault and kind, then shows
   * the Library again, even after a failure that left part of the task's work recorded. The
   * task's vault reports it, even while it reopens; another vault loaded meanwhile neither
   * shows nor reports it.
   */
  async function runVaultLibraryTask<Summary>(task: VaultLibraryTask<Summary>) {
    if (
      loadedVaultRoot === null ||
      openedVaultRoot.current !== loadedVaultRoot ||
      task.runningVaults.current.has(loadedVaultRoot)
    ) {
      return;
    }
    const taskVaultRoot = loadedVaultRoot;
    task.runningVaults.current.add(taskVaultRoot);
    const reportsHere = () => activeVaultRoot.current === taskVaultRoot;
    task.setRunning(true);
    task.setStatus(null);
    setError(null);
    let failure: { reason: unknown } | null = null;
    try {
      const summary = await task.run();
      if (reportsHere()) {
        task.setStatus(task.describe(summary));
      }
    } catch (reason) {
      failure = { reason };
    } finally {
      // The task is over; showing what it recorded is not part of it.
      task.runningVaults.current.delete(taskVaultRoot);
      if (reportsHere()) {
        task.setRunning(false);
      }
    }
    if (!reportsHere()) {
      return;
    }
    if (failure !== null) {
      setError(errorMessage(failure.reason));
    }
    // A reopening of the vault searches the Library itself once the vault is open.
    if (openedVaultRoot.current !== taskVaultRoot) {
      return;
    }
    try {
      await showChangedLibrary(taskVaultRoot);
    } catch (reason) {
      if (reportsHere()) {
        setError(errorMessage((failure ?? { reason }).reason));
      }
      return;
    }
    // A search shown meanwhile cleared the task failure.
    if (failure !== null && reportsHere()) {
      setError(errorMessage(failure.reason));
    }
  }

  /**
   * Imports a reference catalog file into the opened vault, then shows its releases and the
   * records it left awaiting review.
   */
  async function importReferenceCatalog(input: ReferenceImportInput) {
    const importVaultRoot = loadedVaultRoot;
    await runVaultLibraryTask({
      runningVaults: importingReferenceVaults,
      setRunning: setImportingReference,
      setStatus: setReferenceImportStatus,
      run: () => invoke<ReferenceImportSummary>("import_reference_catalog", { input }),
      describe: describeReferenceImport,
    });
    if (importVaultRoot === null) {
      return;
    }
    try {
      await refreshReferenceReviews(importVaultRoot);
    } catch (reason) {
      if (activeVaultRoot.current === importVaultRoot) {
        setError(errorMessage(reason));
      }
    }
  }

  /** Renders the thumbnails the Library lacks, then shows them. */
  function renderThumbnails() {
    return runVaultLibraryTask({
      runningVaults: renderingThumbnailVaults,
      setRunning: setRenderingThumbnails,
      setStatus: setThumbnailStatus,
      run: () =>
        invoke<DerivationSummary>("derive_thumbnails", { max_edge: LIBRARY_THUMBNAIL_EDGE }),
      describe: describeThumbnailRendering,
    });
  }

  /** Builds the packaging models complete releases lack, then shows them. */
  function buildPackagingModels() {
    return runVaultLibraryTask({
      runningVaults: buildingModelVaults,
      setRunning: setBuildingModels,
      setStatus: setModelStatus,
      run: () => invoke<PackagingModelSummary>("derive_packaging_models"),
      describe: describePackagingModels,
    });
  }

  /**
   * Shows the Library again once the vault gained thumbnails or releases: a filter search
   * submitted meanwhile is searched again so its results include them.
   */
  async function showChangedLibrary(changedVaultRoot: string) {
    const pendingFilters = pendingSearchRef.current;
    if (pendingFilters !== null) {
      await applyLibraryFilters(pendingFilters);
      return;
    }
    const generation = supersedeLibraryRequests();
    const isCurrent = () =>
      openedVaultRoot.current === changedVaultRoot && generation === vaultDataGeneration.current;
    try {
      const library = await searchLibrary();
      if (isCurrent()) {
        showLibraryPage(library);
      }
    } catch (reason) {
      // A search or refresh that replaced this one reports for itself.
      if (isCurrent()) {
        throw reason;
      }
    }
  }

  async function showSources() {
    showView("sources");
    void readSourceFailures();
    // Machine settings change outside this view, as from the CLI, so every visit reads the
    // Sources again; the current list stays shown meanwhile.
    sourcesRequest.current += 1;
    const request = sourcesRequest.current;
    setSourcesError(null);
    try {
      const listed = await invoke<SourceDescription[]>("list_sources");
      if (request === sourcesRequest.current) {
        setSources(listed);
      }
    } catch (reason) {
      // Showing the view again reads them again.
      if (request === sourcesRequest.current) {
        setSourcesError(errorMessage(reason));
      }
    }
  }

  /** Enables or disables a Source on this machine, for every vault, and shows the outcome. */
  async function setSourceEnabled(sourceId: string, enabled: boolean) {
    // A read of the Sources started before this change would show their older state.
    sourcesRequest.current += 1;
    const described = await invoke<SourceDescription[]>("set_source_enabled", {
      source_id: sourceId,
      enabled,
    });
    setSources(described);
  }

  /** Stores on this machine one credential a Source needs, for every vault, never showing it. */
  async function setSourceCredential(sourceId: string, field: string, key: string) {
    sourcesRequest.current += 1;
    setSources(
      await invoke<SourceDescription[]>("set_source_api_key", {
        source_id: sourceId,
        field,
        key,
      }),
    );
  }

  /** Forgets one credential this machine stores for a Source. */
  async function clearSourceCredential(sourceId: string, field: string) {
    sourcesRequest.current += 1;
    setSources(
      await invoke<SourceDescription[]>("clear_source_api_key", { source_id: sourceId, field }),
    );
  }

  /** Reads the failures the loaded vault recorded, which executions add to meanwhile. */
  async function readSourceFailures() {
    sourceFailuresRequest.current += 1;
    const request = sourceFailuresRequest.current;
    if (loadedVaultRoot === null || openedVaultRoot.current !== loadedVaultRoot) {
      setSourceFailures(null);
      return;
    }
    try {
      const summaries = await invoke<SourceFailureSummary[]>("list_source_failures", {
        latest: SOURCE_FAILURES_SHOWN,
      });
      if (request === sourceFailuresRequest.current) {
        setSourceFailures(summaries);
      }
    } catch (reason) {
      if (request === sourceFailuresRequest.current) {
        setSourceFailures(null);
        setError(errorMessage(reason));
      }
    }
  }

  async function showRuns() {
    showView("runs");
    const listingLoadGeneration = vaultLoadRequestGeneration.current;
    try {
      await refreshRuns(loadedVaultRoot);
    } catch (reason) {
      // A vault loaded meanwhile reports its own failures.
      if (vaultLoadRequestGeneration.current === listingLoadGeneration) {
        setError(errorMessage(reason));
      }
    }
  }

  // One shared poll follows every executing run of the vault they were started in.
  useEffect(() => {
    if (executingRunIds.size === 0) {
      return;
    }
    const runIds = [...executingRunIds];
    const pollingVaultRoot = activeVaultRoot.current;
    const timer = setInterval(() => {
      refreshExecutingRuns(runIds, pollingVaultRoot).catch(() => {
        // The next poll or the final refresh after the execution reports persistent failures.
      });
    }, RUN_PROGRESS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [executingRunIds]);

  async function startRun(request: AcquisitionRequestDraft) {
    if (loadedVaultRoot === null) {
      setError("Load a vault before starting an acquisition.");
      return;
    }
    const startingVaultRoot = loadedVaultRoot;
    setStartingRun(true);
    setError(null);
    let started: AcquisitionRun;
    try {
      started = await invoke<AcquisitionRun>("start_acquisition_run", { request });
    } catch (reason) {
      if (activeVaultRoot.current === startingVaultRoot) {
        setError(errorMessage(reason));
      }
      return;
    } finally {
      setStartingRun(false);
    }
    if (activeVaultRoot.current !== startingVaultRoot) {
      return;
    }
    // The run is persisted from here on: show it even if the list refresh fails.
    setRuns((current) => [...current.filter((run) => run.id !== started.id), started]);
    showView("runs");
    try {
      await refreshRuns(startingVaultRoot);
    } catch (reason) {
      if (activeVaultRoot.current === startingVaultRoot) {
        setError(
          `Run #${started.id} started, but the run list could not be refreshed: ${errorMessage(reason)}`,
        );
      }
    }
  }

  /** Records whether a run of `vaultRoot` executes, showing it when that vault is active. */
  function trackExecution(vaultRoot: string | null, runId: number, executing: boolean) {
    const runIds = new Set(executionsByVault.current.get(vaultRoot));
    if (executing) {
      runIds.add(runId);
    } else {
      runIds.delete(runId);
    }
    executionsByVault.current.set(vaultRoot, runIds);
    if (activeVaultRoot.current === vaultRoot) {
      setExecutingRunIds(new Set(runIds));
    }
  }

  async function executeRun(runId: number) {
    const actingVaultRoot = loadedVaultRoot;
    // While executing, the shared poll follows the run's counts; the library and Review Items
    // are refreshed once it ends.
    trackExecution(actingVaultRoot, runId, true);
    setError(null);
    let executionError: string | null = null;
    try {
      await invoke<AcquisitionRun>("execute_acquisition_run", {
        run_id: runId,
        matching_policy: MATCHING_POLICY,
      });
    } catch (reason) {
      executionError = errorMessage(reason);
      if (activeVaultRoot.current === actingVaultRoot) {
        setError(executionError);
      }
    } finally {
      trackExecution(actingVaultRoot, runId, false);
    }
    // Executions persist imports, Review Items and progress as they go, even when they fail.
    try {
      await Promise.all([refreshVaultData(actingVaultRoot), refreshRuns(actingVaultRoot)]);
    } catch (reason) {
      if (activeVaultRoot.current === actingVaultRoot) {
        // Why the execution stopped matters more than the failed refresh after it.
        const refreshError = errorMessage(reason);
        setError(
          executionError === null
            ? refreshError
            : `${executionError} (the vault could not be refreshed: ${refreshError})`,
        );
      }
    }
  }

  async function applyRunAction(runId: number, action: RunAction) {
    const actingVaultRoot = loadedVaultRoot;
    // A vault loaded while this action runs tracks its own pending actions.
    const actingLoadGeneration = vaultLoadRequestGeneration.current;
    setBusyRunIds((current) => new Set(current).add(runId));
    setError(null);
    try {
      const updated = await invoke<AcquisitionRun>(`${action}_acquisition_run`, {
        run_id: runId,
      });
      if (activeVaultRoot.current !== actingVaultRoot) {
        return;
      }
      // The transition is persisted from here on: show it even if the list refresh fails.
      setRuns((current) => current.map((run) => (run.id === updated.id ? updated : run)));
      try {
        await refreshRuns(actingVaultRoot);
      } catch (reason) {
        if (activeVaultRoot.current === actingVaultRoot) {
          setError(
            `Run #${runId} is ${updated.status}, but the run list could not be refreshed: ${errorMessage(reason)}`,
          );
        }
      }
    } catch (reason) {
      if (activeVaultRoot.current === actingVaultRoot) {
        setError(errorMessage(reason));
      }
    } finally {
      if (vaultLoadRequestGeneration.current === actingLoadGeneration) {
        setBusyRunIds((current) => withoutRun(current, runId));
      }
    }
  }

  const loadReviewPreview = useCallback(
    async (reviewItemId: number): Promise<ArrayBuffer> => {
      if (loadedVaultRoot === null) {
        throw new Error("Load a vault before loading review previews.");
      }
      const previewVaultRoot = loadedVaultRoot;
      const preview = await invoke<ArrayBuffer>("load_review_preview", {
        review_item_id: reviewItemId,
      });
      if (activeVaultRoot.current !== previewVaultRoot) {
        throw new Error("Vault changed while loading review preview.");
      }
      return preview;
    },
    [loadedVaultRoot],
  );

  return (
    <main className="shell">
      <header className="topbar">
        <div>
          <h1>Game Media Vault</h1>
          <p>Local preservation library</p>
        </div>
        <span className="asset-count">
          {releaseCountLabel} · {reviewCountLabel}
        </span>
      </header>

      <form
        className="vault-picker"
        onSubmit={(event: FormEvent) => {
          event.preventDefault();
          void loadVault(false);
        }}
      >
        <label htmlFor="vault-root">Vault path</label>
        <div className="vault-controls">
          <input
            id="vault-root"
            value={vaultRoot}
            onChange={(event) => setVaultRoot(event.target.value)}
            spellCheck={false}
          />
          <button type="submit" disabled={loading || vaultRoot.trim().length === 0}>
            {loading ? "Loading…" : "Load vault"}
          </button>
          <button
            type="button"
            disabled={loading || vaultRoot.trim().length === 0}
            onClick={() => void loadVault(true)}
          >
            Create vault
          </button>
        </div>
        <p className="hint">Use the same vault path passed to the CLI with --vault.</p>
      </form>

      {error ? <p className="error-message">{error}</p> : null}
      <nav className="view-tabs" aria-label="Vault views">
        <button
          type="button"
          className={activeView === "library" ? "active" : ""}
          onClick={() => showView("library")}
        >
          Library ({libraryTotal})
        </button>
        <button
          type="button"
          className={activeView === "review" ? "active" : ""}
          onClick={() => showView("review")}
        >
          Review ({reviewCount})
        </button>
        <button
          type="button"
          className={activeView === "acquire" ? "active" : ""}
          onClick={() => showView("acquire")}
        >
          Acquire
        </button>
        <button
          type="button"
          className={activeView === "runs" ? "active" : ""}
          onClick={() => void showRuns()}
        >
          Runs
        </button>
        <button
          type="button"
          className={activeView === "sources" ? "active" : ""}
          onClick={() => void showSources()}
        >
          Sources
        </button>
      </nav>

      {activeView === "library" ? (
        <>
          {loadedVaultRoot === null ? null : (
            <ReferenceImportForm
              importing={importingReference}
              status={referenceImportStatus}
              onImport={(input) => void importReferenceCatalog(input)}
            />
          )}
        <LibraryView
          entries={entries}
          objectUrl={originalObjectUrl}
          filters={libraryFilters}
          filtersRevision={libraryFiltersRevision}
          onSearch={
            loadedVaultRoot === null ? undefined : (filters) => void applyLibraryFilters(filters)
          }
          canLoadMore={libraryNextAfter !== null}
          loadingMore={loadingMore}
          searching={searchingLibrary}
          onLoadMore={() => void loadMoreReleases()}
          onRenderThumbnails={
            loadedVaultRoot === null ? undefined : () => void renderThumbnails()
          }
          renderingThumbnails={renderingThumbnails}
          thumbnailStatus={thumbnailStatus}
          onBuildPackagingModels={
            loadedVaultRoot === null ? undefined : () => void buildPackagingModels()
          }
          buildingPackagingModels={buildingModels}
          packagingModelStatus={modelStatus}
        />
        </>
      ) : null}
      {activeView === "review" ? (
        <>
          {referenceReviewItems.length > 0 ? (
            <ReferenceReviewView
              items={referenceReviewItems}
              decidingIds={decidingReferenceIds}
              onLink={(itemId, releaseEditionId) =>
                void decideReferenceReview(itemId, () =>
                  invoke<ReferenceReviewItem[]>("link_reference_review_item", {
                    item_id: itemId,
                    release_edition_id: releaseEditionId,
                  }),
                )
              }
              onKeepApart={(itemId) =>
                void decideReferenceReview(itemId, () =>
                  invoke<ReferenceReviewItem[]>("keep_reference_review_item_apart", {
                    item_id: itemId,
                  }),
                )
              }
            />
          ) : null}
          {reviewItems.length > 0 || referenceReviewItems.length === 0 ? (
            <ReviewView
              items={reviewItems}
              resolvingIds={resolvingIds}
              onResolve={resolveReviewItem}
              onLoadPreview={loadReviewPreview}
            />
          ) : null}
        </>
      ) : null}
      {activeView === "acquire" ? (
        <AcquireView
          starting={startingRun}
          onStart={(request) => void startRun(request)}
          onCheckPlan={(request) => invoke<AcquisitionPlan>("plan_acquisition", { request })}
        />
      ) : null}
      {activeView === "sources" ? (
        <SourcesView
          sources={sources}
          error={sourcesError}
          failures={sourceFailures}
          onSetEnabled={setSourceEnabled}
          onSetApiKey={setSourceCredential}
          onClearApiKey={clearSourceCredential}
        />
      ) : null}
      {activeView === "runs" ? (
        <RunsView
          runs={runs}
          busyRunIds={busyRunIds}
          executingRunIds={executingRunIds}
          onExecute={(runId) => void executeRun(runId)}
          onPause={(runId) => void applyRunAction(runId, "pause")}
          onResume={(runId) => void applyRunAction(runId, "resume")}
          onCancel={(runId) => void applyRunAction(runId, "cancel")}
        />
      ) : null}
    </main>
  );
}

function withoutRun(runIds: Set<number>, runId: number) {
  const next = new Set(runIds);
  next.delete(runId);
  return next;
}

/** Original objects are served by the desktop shell's `gmv-object` protocol. */
function originalObjectUrl(objectHash: string) {
  return convertFileSrc(objectHash, "gmv-object");
}

function describeReferenceImport(summary: ReferenceImportSummary) {
  const imported = `Imported ${summary.imported_releases} ${
    summary.imported_releases === 1 ? "release" : "releases"
  }`;
  if (summary.skipped_records === 0) {
    return `${imported}.`;
  }
  const skipped = summary.skipped_records === 1 ? "record" : "records";
  return `${imported}; ${summary.skipped_records} malformed ${skipped} skipped.`;
}

function describePackagingModels(summary: PackagingModelSummary) {
  const releases = (count: number) => `${count} ${count === 1 ? "release" : "releases"}`;
  const parts = [`Built ${summary.generated} 3D ${summary.generated === 1 ? "box" : "boxes"}`];
  if (summary.up_to_date > 0) {
    parts.push(`${summary.up_to_date} already built`);
  }
  if (summary.incomplete.length > 0) {
    const count = summary.incomplete.length;
    parts.push(`${releases(count)} ${count === 1 ? "misses" : "miss"} scans`);
  }
  if (summary.without_template > 0) {
    const count = summary.without_template;
    parts.push(`${releases(count)} ${count === 1 ? "has" : "have"} no 3D template`);
  }
  if (summary.failed.length > 0) {
    parts.push(`${releases(summary.failed.length)} could not be built`);
  }
  return parts.join("; ") + ".";
}

function describeThumbnailRendering(summary: DerivationSummary) {
  const parts = [
    `Rendered ${summary.derived} ${summary.derived === 1 ? "thumbnail" : "thumbnails"}`,
  ];
  const failed = summary.failed.length;
  if (failed > 0) {
    parts.push(`${failed} ${failed === 1 ? "original" : "originals"} could not be rendered`);
  }
  if (summary.skipped > 0) {
    parts.push(
      summary.skipped === 1
        ? "1 original skipped as an unsupported format"
        : `${summary.skipped} originals skipped as unsupported formats`,
    );
  }
  return parts.join("; ") + ".";
}

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";

import { AcquireView } from "./AcquireView";
import type {
  AcquisitionPlan,
  AcquisitionRequestDraft,
  AcquisitionRun,
  SourceDescription,
  SourceFailureSummary,
} from "./acquisition";
import { ActivityView, type PreparingDownload } from "./ActivityView";
import { consoleName } from "./catalog";
import { Dialog } from "./controls";
import { DownloadView } from "./DownloadView";
import { Icon } from "./icons";
import { LIBRARY_THUMBNAIL_EDGE, LibraryView } from "./LibraryView";
import { ReviewView } from "./ReviewView";
import { SourcesView } from "./SourcesView";
import { ReferenceImportForm } from "./ReferenceImportForm";
import { ReferenceReviewView } from "./ReferenceReviewView";
import { AppShell, Page, VaultForm, type View, Welcome } from "./Shell";
import { NO_LIBRARY_FILTERS, errorMessage } from "./types";
import type {
  DerivationSummary,
  ExportSummary,
  LatestMedium,
  LibraryEntry,
  LibraryFilters,
  LibraryPage,
  PackagingModelSummary,
  ReferenceImportInput,
  ReferenceImportSummary,
  ReferenceReviewItem,
  PendingReviewDecision,
  PendingReviewSummary,
  ReviewDecision,
  ReviewItem,
  ReviewPage,
} from "./types";

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

/** Review Items shown on a page of the Review view. */
const REVIEW_PAGE_SIZE = 25;

/** The media the Activity view shows as just arrived. */
const LATEST_MEDIA_SHOWN = 12;

/** The vault opened last, which the app opens again when it starts. */
const VAULT_ROOT_KEY = "game-media-vault.vault-root";

/** The vault offered on a first launch: a name alone, kept in the user's Documents. */
const DEFAULT_VAULT = "Game Media Vault";

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
  const [vaultRoot, setVaultRoot] = useState(() => rememberedVault() ?? DEFAULT_VAULT);
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
  // The vault the app shows, as typed, from the moment it is asked for: until one is, or while
  // none could be opened yet, the welcome screen shows.
  const [shownVault, setShownVault] = useState<string | null>(null);
  const vaultEverOpened = useRef(false);
  const [changingVault, setChangingVault] = useState(false);
  // Platforms whose games have media, which the Library's console filter offers.
  const [libraryPlatforms, setLibraryPlatforms] = useState<string[]>([]);
  // Whether the shown results extend past their first page, which a live refresh would drop.
  const libraryPaged = useRef(false);
  // Whether media arrived since the shown Library results were read.
  const [newMedia, setNewMedia] = useState(false);
  const [latestMedia, setLatestMedia] = useState<LatestMedium[]>([]);
  // The newest Asset the app saw, so a poll tells when new media arrived: none when the vault
  // held none, unset before the first read.
  const newestAssetSeen = useRef<number | null | undefined>(undefined);
  // Downloads chosen in the Download view start one after another, each run fetching its game
  // list first; their starts chain here.
  const startChain = useRef<Promise<void>>(Promise.resolve());
  const preparingKey = useRef(0);
  // Each with the vault it was chosen in.
  const [preparing, setPreparing] = useState<(PreparingDownload & { vaultRoot: string })[]>([]);
  // Runs waiting to execute, by vault: each vault executes one run at a time, so Sources are
  // never asked for two downloads at once.
  const waitingRuns = useRef(new Map<string, number[]>());
  const drainingVaults = useRef(new Set<string>());
  // Runs resumed while their execution still winds down after a pause, by vault: that
  // execution may already be stopping, so they execute again once it ends.
  const executeAgain = useRef(new Map<string, Set<number>>());
  const [waitingRunIds, setWaitingRunIds] = useState<Set<number>>(() => new Set());
  // Whether a poll is reading the latest media; a slow read is never doubled by the next poll.
  const followingMedia = useRef(false);
  // How many Review Items await a decision in all, and how many come before the page shown.
  const [reviewUndecided, setReviewUndecided] = useState(0);
  const reviewOffset = useRef(0);
  const [shownReviewOffset, setShownReviewOffset] = useState(0);
  // Vaults deciding every Review Item: the backend goes on while another vault is loaded, so
  // loading the vault again shows the decision under way instead of offering another.
  const decidingReviewVaults = useRef(new Set<string>());
  // The decision on every Review Item of the open vault under way, or what the last one did.
  const [bulkReview, setBulkReview] = useState<{ deciding: boolean; outcome: string | null }>({
    deciding: false,
    outcome: null,
  });
  const reviewCount = reviewUndecided + referenceReviewItems.length;

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
        // Games without media show only when asked: a vault holds every game of each console
        // downloaded, most of them without media for a while.
        asset_types:
          filters.assetTypes.length > 0 || filters.includeWithoutMedia
            ? filters.assetTypes
            : ["any"],
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
    libraryPaged.current = false;
    setNewMedia(false);
    if (page.platforms_with_media !== undefined) {
      setLibraryPlatforms(page.platforms_with_media);
    }
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
        libraryPaged.current = true;
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

  /**
   * Opens the vault at `requestedVaultRoot`, the one typed by default; with `create`, a folder
   * holding no vault gets a new one.
   */
  async function loadVault(create: boolean, requestedVaultRoot = vaultRoot) {
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
    setShownVault(requestedVaultRoot);
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
    setReviewUndecided(0);
    reviewOffset.current = 0;
    setShownReviewOffset(0);
    setBulkReview({ deciding: decidingReviewVaults.current.has(vaultKey), outcome: null });
    referenceReviewGeneration.current += 1;
    setReferenceReviewItems([]);
    setDecidingReferenceIds(new Set());
    setLoadedVaultRoot(null);
    setResolvingIds(new Set());
    setRuns([]);
    setBusyRunIds(new Set());
    setExecutingRunIds(new Set(executionsByVault.current.get(vaultKey)));
    setWaitingRunIds(new Set(waitingRuns.current.get(vaultKey)));
    // Downloads still preparing belong to the vault they were chosen in, which stops them;
    // those that could not start stay with their vault until dismissed.
    setPreparing((current) => current.filter((item) => item.failure !== undefined));
    setLatestMedia([]);
    newestAssetSeen.current = undefined;
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
      remember(requestedVaultRoot);
      if (identity !== vaultKey) {
        vaultKey = identity;
        activeVaultRoot.current = identity;
        setRenderingThumbnails(renderingThumbnailVaults.current.has(identity));
        setBuildingModels(buildingModelVaults.current.has(identity));
        setImportingReference(importingReferenceVaults.current.has(identity));
        setBulkReview({ deciding: decidingReviewVaults.current.has(identity), outcome: null });
        setExecutingRunIds(new Set(executionsByVault.current.get(identity)));
        setWaitingRunIds(new Set(waitingRuns.current.get(identity)));
      }
      openedVaultRoot.current = vaultKey;
      vaultEverOpened.current = true;
      setChangingVault(false);
      // Runs left waiting while another vault was open execute again.
      void drainExecutions(vaultKey);
      // A refresh started after this search, such as one after a rendering, shows newer data.
      const libraryGeneration = supersedeLibraryRequests();
      const [library, reviews] = await Promise.all([
        searchLibrary(),
        readReviewPage(),
        refreshReferenceReviews(vaultKey),
        // What the vault held when it opened, so media arriving later tell apart.
        refreshLatestMedia(vaultKey).catch(() => false),
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
        showReviewPage(reviews);
      }
      setLoadedVaultRoot(vaultKey);
      if (activeViewRef.current === "activity") {
        await Promise.all([refreshRuns(vaultKey), refreshLatestMedia(vaultKey)]);
      }
    } catch (reason) {
      if (
        activeVaultRoot.current === vaultKey &&
        loadGeneration === vaultLoadRequestGeneration.current
      ) {
        setError(errorMessage(reason));
        // A first vault that cannot be opened leaves the welcome screen to try another.
        if (!vaultEverOpened.current) {
          setShownVault(null);
        }
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

  /**
   * Reads the page of the Review Items awaiting a decision the Review view shows, or the last
   * page when decisions left fewer items than that page starts at.
   */
  async function readReviewPage() {
    const offset = reviewOffset.current;
    const page = await invoke<ReviewPage>("review_page", { offset, limit: REVIEW_PAGE_SIZE });
    if (page.items.length > 0 || offset === 0 || offset < page.undecided) {
      return page;
    }
    // Unless another page was asked for meanwhile, which a later read shows.
    if (reviewOffset.current === offset) {
      reviewOffset.current =
        Math.max(0, Math.ceil(page.undecided / REVIEW_PAGE_SIZE) - 1) * REVIEW_PAGE_SIZE;
    }
    return invoke<ReviewPage>("review_page", {
      offset: reviewOffset.current,
      limit: REVIEW_PAGE_SIZE,
    });
  }

  function showReviewPage(page: ReviewPage) {
    setReviewItems(page.items);
    setReviewUndecided(page.undecided);
    setShownReviewOffset(page.offset);
  }

  /** Shows the page of Review Items after the first `offset`, or the last page past the end. */
  async function showReviewOffset(offset: number) {
    if (loadedVaultRoot === null || openedVaultRoot.current !== loadedVaultRoot) {
      return;
    }
    const readingVaultRoot = loadedVaultRoot;
    // The page shown changes once the asked one is read; until then the shown one stays.
    const shownOffset = reviewOffset.current;
    reviewOffset.current = offset;
    reviewRefreshRequestGeneration.current += 1;
    const generation = reviewRefreshRequestGeneration.current;
    try {
      const page = await readReviewPage();
      if (
        activeVaultRoot.current === readingVaultRoot &&
        generation === reviewRefreshRequestGeneration.current
      ) {
        showReviewPage(page);
      }
    } catch (reason) {
      if (activeVaultRoot.current === readingVaultRoot) {
        if (generation === reviewRefreshRequestGeneration.current) {
          reviewOffset.current = shownOffset;
        }
        setError(errorMessage(reason));
      }
    }
  }

  /**
   * Decides every Review Item awaiting a decision at once, then shows the first page, the
   * Library the decisions changed and the runs they reopened.
   */
  async function decideAllReviews(decision: PendingReviewDecision) {
    if (loadedVaultRoot === null) {
      setError("Open a vault before deciding reviews.");
      return;
    }
    if (
      openedVaultRoot.current !== loadedVaultRoot ||
      decidingReviewVaults.current.has(loadedVaultRoot)
    ) {
      return;
    }
    const decidingVaultRoot = loadedVaultRoot;
    decidingReviewVaults.current.add(decidingVaultRoot);
    setBulkReview({ deciding: true, outcome: null });
    let outcome: string;
    try {
      const summary = await invoke<PendingReviewSummary>("decide_pending_reviews", {
        decision,
        matching_policy: MATCHING_POLICY,
      });
      outcome =
        summary.left === 0
          ? `Decided ${summary.decided}.`
          : `Decided ${summary.decided}; ${summary.left} left for you.`;
    } catch (reason) {
      // The batches recorded before the failure stay decided, which the refresh shows.
      outcome = errorMessage(reason);
    } finally {
      decidingReviewVaults.current.delete(decidingVaultRoot);
    }
    if (activeVaultRoot.current !== decidingVaultRoot) {
      return;
    }
    // A reopening of the vault reads its reviews, Library and runs itself once it is open.
    if (openedVaultRoot.current === decidingVaultRoot) {
      reviewMutationGeneration.current += 1;
      reviewOffset.current = 0;
      setShownReviewOffset(0);
      try {
        await Promise.all([refreshVaultData(decidingVaultRoot), refreshRuns(decidingVaultRoot)]);
      } catch (reason) {
        if (activeVaultRoot.current === decidingVaultRoot) {
          setError(errorMessage(reason));
        }
      }
    }
    if (activeVaultRoot.current === decidingVaultRoot) {
      setBulkReview({ deciding: false, outcome });
    }
  }

  async function resolveReviewItem(reviewItemId: number, decision: ReviewDecision) {
    if (loadedVaultRoot === null) {
      setError("Open a vault before resolving review items.");
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
        readReviewPage(),
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
      showReviewPage(reviews);
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
            readReviewPage(),
            searchLibrary(),
          ]);
          if (
            activeVaultRoot.current === resolvingVaultRoot &&
            refusalRefreshGeneration === reviewRefreshRequestGeneration.current
          ) {
            showReviewPage(reviews);
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

  /** Shows `view`, reading what it shows that may have changed since. */
  function navigate(view: View) {
    if (view === "activity") {
      void showActivity();
    } else if (view === "download") {
      showView("download");
      void readSources();
    } else if (view === "sources") {
      void showSources();
    } else {
      showView(view);
    }
  }

  function showView(view: View) {
    activeViewRef.current = view;
    setActiveView(view);
    // Executions import as they go: the Library shows what they imported so far when it opens,
    // rather than only once they end. Results someone browses stay, and the poll offers the
    // media arriving through the banner instead.
    if (view === "library" && executingRunIds.size > 0 && !browsingLibrary()) {
      const refreshingVaultRoot = activeVaultRoot.current;
      refreshVaultData(refreshingVaultRoot).catch((reason) => {
        if (activeVaultRoot.current === refreshingVaultRoot) {
          setError(errorMessage(reason));
        }
      });
    }
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

  /**
   * Reads the media the opened vault `expectedVaultRoot` retained last, and tells whether newer
   * media arrived since the previous read.
   */
  async function refreshLatestMedia(expectedVaultRoot: string | null) {
    if (expectedVaultRoot === null || openedVaultRoot.current !== expectedVaultRoot) {
      return false;
    }
    const latest = await invoke<LatestMedium[]>("latest_media", { limit: LATEST_MEDIA_SHOWN });
    if (activeVaultRoot.current !== expectedVaultRoot) {
      return false;
    }
    setLatestMedia(latest);
    const newest = latest.at(0)?.asset.asset_id ?? null;
    const previous = newestAssetSeen.current;
    newestAssetSeen.current = newest;
    return previous !== undefined && newest !== null && (previous === null || newest > previous);
  }

  /**
   * Shows the media executions retained since the last poll: in the Activity view, and in the
   * Library, at once while it shows a first page alone, else behind its banner, so the results
   * someone pages through or searches never change under them.
   */
  async function followArrivingMedia(expectedVaultRoot: string | null) {
    if (followingMedia.current) {
      return;
    }
    followingMedia.current = true;
    try {
      const arrived = await refreshLatestMedia(expectedVaultRoot);
      if (!arrived || activeVaultRoot.current !== expectedVaultRoot) {
        return;
      }
      if (browsingLibrary()) {
        setNewMedia(true);
        return;
      }
      await refreshVaultData(expectedVaultRoot);
    } catch {
      // The next poll, or the refresh once the execution ends, shows them.
    } finally {
      followingMedia.current = false;
    }
  }

  /** Whether the Library shows more than a first page, or a search or page is pending. */
  function browsingLibrary() {
    return (
      libraryPaged.current || pendingSearchRef.current !== null || pageRequestRef.current !== null
    );
  }

  /** Reads the Review Items of `expectedVaultRoot` again, unless a newer read or a decision did. */
  async function refreshReviewItems(expectedVaultRoot: string | null) {
    if (openedVaultRoot.current !== expectedVaultRoot) {
      return;
    }
    reviewRefreshRequestGeneration.current += 1;
    const generation = reviewRefreshRequestGeneration.current;
    const decisions = reviewMutationGeneration.current;
    const reviews = await readReviewPage();
    if (
      activeVaultRoot.current === expectedVaultRoot &&
      generation === reviewRefreshRequestGeneration.current &&
      decisions === reviewMutationGeneration.current
    ) {
      showReviewPage(reviews);
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
      readReviewPage(),
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
      showReviewPage(reviews);
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
  /** Copies the loaded vault's originals to `destination`, a folder people browse. */
  async function exportLibrary(destination: string) {
    return invoke<ExportSummary>("export_library", { destination, platforms: [] });
  }

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
    await readSources();
  }

  /**
   * Reads the registered Sources, which the Sources and Download views show. Machine settings
   * change outside the app, as from the CLI, so every visit reads them again; the current list
   * stays shown meanwhile.
   */
  async function readSources() {
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

  async function showActivity() {
    showView("activity");
    const listingLoadGeneration = vaultLoadRequestGeneration.current;
    const listingVaultRoot = loadedVaultRoot;
    try {
      const [, arrived] = await Promise.all([
        refreshRuns(listingVaultRoot),
        refreshLatestMedia(listingVaultRoot),
      ]);
      // Media this read sees first would never reach a Library someone browses otherwise.
      if (arrived && activeVaultRoot.current === listingVaultRoot && browsingLibrary()) {
        setNewMedia(true);
      }
    } catch (reason) {
      // A vault loaded meanwhile reports its own failures.
      if (vaultLoadRequestGeneration.current === listingLoadGeneration) {
        setError(errorMessage(reason));
      }
    }
  }

  /** Opens the folder picked with the desktop's own dialog as the vault. */
  async function chooseVaultFolder() {
    const picked = await pickFolder({ directory: true, title: "Choose your vault folder" });
    if (typeof picked === "string") {
      setVaultRoot(picked);
      await loadVault(true, picked);
    }
  }

  // The vault opened last opens again; one gone since is reported rather than created anew.
  useEffect(() => {
    const remembered = rememberedVault();
    if (remembered !== null) {
      void loadVault(false, remembered);
    }
  }, []);

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
      void followArrivingMedia(pollingVaultRoot);
    }, RUN_PROGRESS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [executingRunIds]);

  /**
   * Starts a download of each request, one after another, then executes them in turn; the
   * Activity view follows them meanwhile. Each start fetches the game list of its consoles
   * first, which takes a moment.
   */
  function startDownloads(requests: AcquisitionRequestDraft[]) {
    if (loadedVaultRoot === null) {
      setError("Open a vault before downloading.");
      return;
    }
    const startingVaultRoot = loadedVaultRoot;
    const downloads = requests.map((request) => {
      preparingKey.current += 1;
      return {
        request,
        preparing: {
          key: preparingKey.current,
          title: requestTitle(request),
          vaultRoot: startingVaultRoot,
        },
      };
    });
    setPreparing((current) => [...current, ...downloads.map((download) => download.preparing)]);
    setError(null);
    void showActivity();
    for (const download of downloads) {
      startChain.current = startChain.current.then(() =>
        startDownload(startingVaultRoot, download.request, download.preparing),
      );
    }
  }

  /** Starts the run of one download chosen in `startingVaultRoot`, then queues its execution. */
  async function startDownload(
    startingVaultRoot: string,
    request: AcquisitionRequestDraft,
    download: PreparingDownload,
  ) {
    const settle = () =>
      setPreparing((current) => current.filter((item) => item.key !== download.key));
    // Another vault opened meanwhile: its runs are not this download's.
    if (openedVaultRoot.current !== startingVaultRoot) {
      return;
    }
    let started: AcquisitionRun;
    try {
      started = await invoke<AcquisitionRun>("start_acquisition_run", { request });
    } catch (reason) {
      // The download stays in the Activity view with its reason, beside any other that failed,
      // even when its vault opened again meanwhile and dropped the cards still preparing.
      if (activeVaultRoot.current === startingVaultRoot) {
        const failed = { ...download, vaultRoot: startingVaultRoot, failure: errorMessage(reason) };
        setPreparing((current) =>
          current.some((item) => item.key === download.key)
            ? current.map((item) => (item.key === download.key ? failed : item))
            : [...current, failed],
        );
      }
      return;
    }
    if (activeVaultRoot.current !== startingVaultRoot) {
      return;
    }
    settle();
    // The run is persisted from here on: show it even if the list refresh fails.
    setRuns((current) => [...current.filter((run) => run.id !== started.id), started]);
    queueExecution(startingVaultRoot, started.id);
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

  /** Shows the runs of `vaultRoot` waiting to execute, when that vault is the active one. */
  function showWaiting(vaultRoot: string) {
    if (activeVaultRoot.current === vaultRoot) {
      setWaitingRunIds(new Set(waitingRuns.current.get(vaultRoot)));
    }
  }

  /** Executes run `runId` of `vaultRoot` once the runs queued before it executed. */
  function queueExecution(vaultRoot: string, runId: number) {
    const queue = waitingRuns.current.get(vaultRoot) ?? [];
    if (executionsByVault.current.get(vaultRoot)?.has(runId)) {
      executeAgain.current.set(
        vaultRoot,
        new Set(executeAgain.current.get(vaultRoot)).add(runId),
      );
    } else if (!queue.includes(runId)) {
      waitingRuns.current.set(vaultRoot, [...queue, runId]);
      showWaiting(vaultRoot);
    }
    void drainExecutions(vaultRoot);
  }

  /** Forgets that run `runId` of `vaultRoot` waits to execute, as once it is paused. */
  function dequeueExecution(vaultRoot: string, runId: number) {
    const queue = waitingRuns.current.get(vaultRoot) ?? [];
    waitingRuns.current.set(
      vaultRoot,
      queue.filter((queued) => queued !== runId),
    );
    executeAgain.current.get(vaultRoot)?.delete(runId);
    showWaiting(vaultRoot);
  }

  /**
   * Executes the waiting runs of `vaultRoot` one after another while it is the open vault: run
   * ids name other runs in another vault, so its runs wait until it opens again.
   */
  async function drainExecutions(vaultRoot: string) {
    if (drainingVaults.current.has(vaultRoot)) {
      return;
    }
    drainingVaults.current.add(vaultRoot);
    try {
      for (;;) {
        const [next, ...rest] = waitingRuns.current.get(vaultRoot) ?? [];
        if (next === undefined || openedVaultRoot.current !== vaultRoot) {
          return;
        }
        waitingRuns.current.set(vaultRoot, rest);
        showWaiting(vaultRoot);
        await executeRun(next, vaultRoot);
      }
    } finally {
      drainingVaults.current.delete(vaultRoot);
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

  /**
   * Executes run `runId` of `actingVaultRoot`, then shows what it persisted; the next waiting run
   * executes as soon as this one ends, without waiting for that refresh.
   */
  async function executeRun(runId: number, actingVaultRoot: string | null) {
    // While executing, the shared poll follows the run's counts; the library and Review Items
    // are refreshed once it ends. A failure of an earlier execution stays shown meanwhile.
    trackExecution(actingVaultRoot, runId, true);
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
    // A resume that came while this execution stopped for a pause executes the run again,
    // after the runs already waiting.
    if (actingVaultRoot !== null && executeAgain.current.get(actingVaultRoot)?.delete(runId)) {
      queueExecution(actingVaultRoot, runId);
    }
    void showExecuted(actingVaultRoot, executionError);
  }

  /**
   * Shows what an execution persisted as it went: imports, Review Items and progress, even when
   * it failed with `executionError`.
   */
  async function showExecuted(actingVaultRoot: string | null, executionError: string | null) {
    try {
      if (browsingLibrary()) {
        // Results someone pages through or searches never change under them: a banner offers
        // the media the execution brought instead.
        const [arrived] = await Promise.all([
          refreshLatestMedia(actingVaultRoot),
          refreshReviewItems(actingVaultRoot),
          refreshRuns(actingVaultRoot),
        ]);
        if (arrived && activeVaultRoot.current === actingVaultRoot) {
          setNewMedia(true);
        }
        return;
      }
      // An execution shorter than a progress poll shows its media here first.
      await Promise.all([
        refreshVaultData(actingVaultRoot),
        refreshRuns(actingVaultRoot),
        refreshLatestMedia(actingVaultRoot),
      ]);
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
      if (actingVaultRoot !== null) {
        // A paused or cancelled run no longer waits to execute; a resumed one downloads again.
        if (action === "resume") {
          queueExecution(actingVaultRoot, runId);
        } else {
          dequeueExecution(actingVaultRoot, runId);
        }
      }
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
        throw new Error("Open a vault before loading review previews.");
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

  const vaultForm = (
    <VaultForm
      vaultRoot={vaultRoot}
      loading={loading}
      onChange={setVaultRoot}
      onOpen={() => void loadVault(true)}
      onChoose={() => void chooseVaultFolder()}
    />
  );

  if (shownVault === null) {
    return <Welcome error={error}>{vaultForm}</Welcome>;
  }

  const vaultReady = loadedVaultRoot !== null;
  // Consoles that could not start are no downloads under way.
  const activeDownloads =
    executingRunIds.size +
    waitingRunIds.size +
    preparing.filter((item) => item.failure === undefined && item.vaultRoot === loadedVaultRoot)
      .length;

  return (
    <AppShell
      view={activeView}
      onNavigate={navigate}
      libraryCount={libraryTotal}
      reviewCount={reviewCount}
      activeDownloads={activeDownloads}
      vaultRoot={shownVault}
      opening={loading}
      onChangeVault={() => setChangingVault(true)}
    >
      {error && !changingVault ? (
        <div className="error-banner" role="alert">
          <span>{error}</span>
          <button
            type="button"
            className="ghost icon-button"
            aria-label="Dismiss"
            onClick={() => setError(null)}
          >
            <Icon name="close" size={16} />
          </button>
        </div>
      ) : null}

      {activeView === "library" ? (
        <Page title="Library" subtitle="Every game of your vault, with the media it holds.">
          <LibraryView
            entries={entries}
            total={libraryTotal}
            objectUrl={originalObjectUrl}
            filters={libraryFilters}
            filtersRevision={libraryFiltersRevision}
            platforms={libraryPlatforms}
            onSearch={vaultReady ? (filters) => void applyLibraryFilters(filters) : undefined}
            canLoadMore={libraryNextAfter !== null}
            loadingMore={loadingMore}
            searching={searchingLibrary}
            onLoadMore={() => void loadMoreReleases()}
            onExport={vaultReady ? exportLibrary : undefined}
            onDownload={() => navigate("download")}
            newMedia={newMedia}
            onRefresh={
              loadedVaultRoot === null
                ? undefined
                : () =>
                    showChangedLibrary(loadedVaultRoot).catch((reason) =>
                      setError(errorMessage(reason)),
                    )
            }
          />
        </Page>
      ) : null}

      {activeView === "download" ? (
        <Page
          title="Download"
          subtitle="Pick consoles and what to collect: the app finds their games and every medium."
        >
          <DownloadView
            sources={sources}
            sourcesError={sourcesError}
            onStart={startDownloads}
            advanced={
              <AcquireView
                starting={false}
                onStart={(request) => startDownloads([request])}
                onCheckPlan={(request) => invoke<AcquisitionPlan>("plan_acquisition", { request })}
              />
            }
          />
        </Page>
      ) : null}

      {activeView === "activity" ? (
        <Page title="Activity" subtitle="Downloads under way and done, as media arrive.">
          <ActivityView
            runs={runs}
            preparing={preparing.filter((item) => item.vaultRoot === loadedVaultRoot)}
            executingRunIds={executingRunIds}
            waitingRunIds={waitingRunIds}
            busyRunIds={busyRunIds}
            latest={latestMedia}
            objectUrl={originalObjectUrl}
            onContinue={(runId) => {
              if (loadedVaultRoot !== null) {
                queueExecution(loadedVaultRoot, runId);
              }
            }}
            onPause={(runId) => void applyRunAction(runId, "pause")}
            onResume={(runId) => void applyRunAction(runId, "resume")}
            onCancel={(runId) => void applyRunAction(runId, "cancel")}
            onDismiss={(key) =>
              setPreparing((current) => current.filter((item) => item.key !== key))
            }
          />
        </Page>
      ) : null}

      {activeView === "review" ? (
        <Page title="Review" subtitle="Media the app was unsure about: confirm or reject them.">
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
          {reviewItems.length > 0 ||
          referenceReviewItems.length === 0 ||
          bulkReview.deciding ||
          bulkReview.outcome !== null ? (
            <ReviewView
              items={reviewItems}
              undecided={reviewUndecided}
              offset={shownReviewOffset}
              pageSize={REVIEW_PAGE_SIZE}
              onPage={(offset) => void showReviewOffset(offset)}
              resolvingIds={resolvingIds}
              onResolve={resolveReviewItem}
              onLoadPreview={loadReviewPreview}
              onDecideAll={vaultReady ? (decision) => void decideAllReviews(decision) : undefined}
              decidingAll={bulkReview.deciding}
              decideAllOutcome={bulkReview.outcome}
            />
          ) : null}
        </Page>
      ) : null}

      {activeView === "sources" ? (
        <Page
          title="Sources"
          subtitle="Where media come from. Some need a free key, stored on this machine only."
        >
          <SourcesView
            sources={sources}
            error={sourcesError}
            failures={sourceFailures}
            onSetEnabled={setSourceEnabled}
            onSetApiKey={setSourceCredential}
            onClearApiKey={clearSourceCredential}
          />
        </Page>
      ) : null}

      {activeView === "settings" ? (
        <Page title="Settings" subtitle="The vault and the tools that tidy its library.">
          <section className="card settings-section" aria-label="Vault">
            <h2>Vault</h2>
            <p className="hint vault-path">{shownVault}</p>
            <button type="button" onClick={() => setChangingVault(true)}>
              <Icon name="folder" size={18} />
              Change vault…
            </button>
          </section>
          <section className="card settings-section" aria-label="Library tools">
            <h2>Library tools</h2>
            <div className="tool-row">
              <div>
                <strong>Thumbnails</strong>
                <p className="hint">Small copies of the images, which make the library faster.</p>
                {thumbnailStatus ? <p className="tool-status">{thumbnailStatus}</p> : null}
              </div>
              <button
                type="button"
                disabled={!vaultReady || renderingThumbnails}
                onClick={() => void renderThumbnails()}
              >
                {renderingThumbnails ? "Rendering thumbnails…" : "Render thumbnails"}
              </button>
            </div>
            <div className="tool-row">
              <div>
                <strong>3D boxes</strong>
                <p className="hint">
                  Boxes built from the front, back and spine scans of complete games.
                </p>
                {modelStatus ? <p className="tool-status">{modelStatus}</p> : null}
              </div>
              <button
                type="button"
                disabled={!vaultReady || buildingModels}
                onClick={() => void buildPackagingModels()}
              >
                {buildingModels ? "Building 3D boxes…" : "Build 3D boxes"}
              </button>
            </div>
          </section>
          <section className="card settings-section" aria-label="Reference catalogs">
            <h2>Reference catalogs</h2>
            <p className="hint">
              Import a No-Intro, Redump or MAME list to name and check your games precisely.
            </p>
            {vaultReady ? (
              <ReferenceImportForm
                importing={importingReference}
                status={referenceImportStatus}
                onImport={(input) => void importReferenceCatalog(input)}
              />
            ) : null}
          </section>
        </Page>
      ) : null}

      {changingVault ? (
        <Dialog title="Open a vault" onClose={() => setChangingVault(false)}>
          {vaultForm}
          {error ? <p className="error-message">{error}</p> : null}
        </Dialog>
      ) : null}
    </AppShell>
  );
}


/** What a download is of, as the Activity view names it: its consoles, else its games. */
function requestTitle(request: AcquisitionRequestDraft) {
  return request.platforms.length > 0
    ? request.platforms.map(consoleName).join(" + ")
    : "Chosen games";
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

/** The vault opened last, or none on a first launch or in a webview that keeps no storage. */
function rememberedVault(): string | null {
  try {
    return localStorage.getItem(VAULT_ROOT_KEY);
  } catch {
    return null;
  }
}

function remember(vaultRoot: string) {
  try {
    localStorage.setItem(VAULT_ROOT_KEY, vaultRoot);
  } catch {
    // A webview that keeps no storage simply opens the default vault next time.
  }
}

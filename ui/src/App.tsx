import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { FormEvent, useCallback, useEffect, useRef, useState } from "react";

import { AcquireView } from "./AcquireView";
import type { AcquisitionRequestDraft, AcquisitionRun } from "./acquisition";
import { LibraryView } from "./LibraryView";
import { ReviewView } from "./ReviewView";
import { RunsView } from "./RunsView";
import { errorMessage } from "./types";
import type { LibraryEntry, ReviewDecision, ReviewItem } from "./types";

type View = "library" | "review" | "acquire" | "runs";

type RunAction = "pause" | "resume" | "cancel";

/** Default thresholds used by desktop executions (SPEC §10 keeps them configurable). */
const MATCHING_POLICY = { high_confidence_threshold: 80, medium_confidence_threshold: 50 };

/** How often run counts are refreshed while a run executes. */
export const RUN_PROGRESS_REFRESH_MS = 3000;

export function App() {
  const activeVaultRoot = useRef<string | null>(null);
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
  const [reviewItems, setReviewItems] = useState<ReviewItem[]>([]);
  const [activeView, setActiveView] = useState<View>("library");
  const [loading, setLoading] = useState(false);
  const [resolvingIds, setResolvingIds] = useState<Set<number>>(() => new Set());
  const [runs, setRuns] = useState<AcquisitionRun[]>([]);
  const [busyRunIds, setBusyRunIds] = useState<Set<number>>(() => new Set());
  const [executingRunIds, setExecutingRunIds] = useState<Set<number>>(() => new Set());
  const [startingRun, setStartingRun] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const releaseCountLabel = `${entries.length} ${entries.length === 1 ? "release" : "releases"}`;
  const reviewCountLabel = `${reviewItems.length} ${reviewItems.length === 1 ? "review" : "reviews"}`;

  async function loadVault(create: boolean) {
    const requestedVaultRoot = vaultRoot;
    activeVaultRoot.current = requestedVaultRoot;
    openedVaultRoot.current = null;
    vaultLoadRequestGeneration.current += 1;
    const loadGeneration = vaultLoadRequestGeneration.current;
    const reviewGenerationAtLoadStart = reviewMutationGeneration.current;
    reviewRefreshRequestGeneration.current += 1;
    setLoading(true);
    setError(null);
    setEntries([]);
    setReviewItems([]);
    setLoadedVaultRoot(null);
    setResolvingIds(new Set());
    setRuns([]);
    setBusyRunIds(new Set());
    setExecutingRunIds(new Set(executionsByVault.current.get(requestedVaultRoot)));
    try {
      // The backend keeps the opened vault; later commands never send a path.
      await invoke("open_vault", { vault_root: requestedVaultRoot, create });
      if (loadGeneration !== vaultLoadRequestGeneration.current) {
        return;
      }
      openedVaultRoot.current = requestedVaultRoot;
      const [library, reviews] = await Promise.all([
        invoke<LibraryEntry[]>("list_library"),
        invoke<ReviewItem[]>("list_review_items"),
      ]);
      if (
        activeVaultRoot.current !== requestedVaultRoot ||
        loadGeneration !== vaultLoadRequestGeneration.current
      ) {
        return;
      }
      // A decision made meanwhile refreshes both lists itself; this load read them before it.
      if (reviewMutationGeneration.current === reviewGenerationAtLoadStart) {
        setEntries(library);
        setReviewItems(reviews);
      }
      setLoadedVaultRoot(requestedVaultRoot);
      if (activeViewRef.current === "runs") {
        await refreshRuns(requestedVaultRoot);
      }
    } catch (reason) {
      if (
        activeVaultRoot.current === requestedVaultRoot &&
        loadGeneration === vaultLoadRequestGeneration.current
      ) {
        setError(errorMessage(reason));
      }
    } finally {
      if (
        activeVaultRoot.current === requestedVaultRoot &&
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
      // Decisions can attach or detach the candidate's asset, so the library is refreshed too.
      const [reviews, library] = await Promise.all([
        invoke<ReviewItem[]>("list_review_items"),
        invoke<LibraryEntry[]>("list_library"),
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
      setEntries(library);
    } catch (reason) {
      if (activeVaultRoot.current === resolvingVaultRoot) {
        setError(errorMessage(reason));
        // A refused decision usually means the item changed elsewhere, possibly linking or
        // detaching its asset; show the current reviews and library.
        reviewRefreshRequestGeneration.current += 1;
        const refusalRefreshGeneration = reviewRefreshRequestGeneration.current;
        try {
          const [reviews, library] = await Promise.all([
            invoke<ReviewItem[]>("list_review_items"),
            invoke<LibraryEntry[]>("list_library"),
          ]);
          if (
            activeVaultRoot.current === resolvingVaultRoot &&
            refusalRefreshGeneration === reviewRefreshRequestGeneration.current
          ) {
            setReviewItems(reviews);
            setEntries(library);
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
    vaultDataGeneration.current += 1;
    const generation = vaultDataGeneration.current;
    const reviewGenerationAtStart = reviewMutationGeneration.current;
    const [library, reviews] = await Promise.all([
      invoke<LibraryEntry[]>("list_library"),
      invoke<ReviewItem[]>("list_review_items"),
    ]);
    // A newer refresh or a review decision made meanwhile read both lists after this one.
    if (
      activeVaultRoot.current === expectedVaultRoot &&
      generation === vaultDataGeneration.current &&
      reviewMutationGeneration.current === reviewGenerationAtStart
    ) {
      setEntries(library);
      setReviewItems(reviews);
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
          Library ({entries.length})
        </button>
        <button
          type="button"
          className={activeView === "review" ? "active" : ""}
          onClick={() => showView("review")}
        >
          Review ({reviewItems.length})
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
      </nav>

      {activeView === "library" ? <LibraryView entries={entries} objectUrl={originalObjectUrl} /> : null}
      {activeView === "review" ? (
        <ReviewView
          items={reviewItems}
          resolvingIds={resolvingIds}
          onResolve={resolveReviewItem}
          onLoadPreview={loadReviewPreview}
        />
      ) : null}
      {activeView === "acquire" ? (
        <AcquireView starting={startingRun} onStart={(request) => void startRun(request)} />
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

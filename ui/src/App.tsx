import { invoke } from "@tauri-apps/api/core";
import { FormEvent, useCallback, useRef, useState } from "react";

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

export function App() {
  const activeVaultRoot = useRef<string | null>(null);
  const vaultLoadRequestGeneration = useRef(0);
  const reviewMutationGeneration = useRef(0);
  const reviewRefreshRequestGeneration = useRef(0);
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
    setExecutingRunIds(new Set());
    try {
      // The backend keeps the opened vault; later commands never send a path.
      await invoke("open_vault", { vault_root: requestedVaultRoot, create });
      if (loadGeneration !== vaultLoadRequestGeneration.current) {
        return;
      }
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
      if (activeView === "runs") {
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

  async function refreshRuns(expectedVaultRoot: string | null) {
    const listed = await invoke<AcquisitionRun[]>("list_acquisition_runs");
    if (activeVaultRoot.current === expectedVaultRoot) {
      setRuns(listed);
    }
  }

  async function refreshVaultData(expectedVaultRoot: string | null) {
    const reviewGenerationAtStart = reviewMutationGeneration.current;
    const [library, reviews] = await Promise.all([
      invoke<LibraryEntry[]>("list_library"),
      invoke<ReviewItem[]>("list_review_items"),
    ]);
    // A review decision made meanwhile refreshed both lists after this read.
    if (
      activeVaultRoot.current === expectedVaultRoot &&
      reviewMutationGeneration.current === reviewGenerationAtStart
    ) {
      setEntries(library);
      setReviewItems(reviews);
    }
  }

  async function showRuns() {
    setActiveView("runs");
    try {
      await refreshRuns(loadedVaultRoot);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }

  async function startRun(request: AcquisitionRequestDraft) {
    if (loadedVaultRoot === null) {
      setError("Load a vault before starting an acquisition.");
      return;
    }
    const startingVaultRoot = loadedVaultRoot;
    setStartingRun(true);
    setError(null);
    try {
      await invoke<AcquisitionRun>("start_acquisition_run", { request });
      if (activeVaultRoot.current !== startingVaultRoot) {
        return;
      }
      setActiveView("runs");
      await refreshRuns(startingVaultRoot);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setStartingRun(false);
    }
  }

  async function executeRun(runId: number) {
    const actingVaultRoot = loadedVaultRoot;
    // A vault loaded while this execution runs tracks its own executions.
    const actingLoadGeneration = vaultLoadRequestGeneration.current;
    setExecutingRunIds((current) => new Set(current).add(runId));
    setError(null);
    try {
      await invoke<AcquisitionRun>("execute_acquisition_run", {
        run_id: runId,
        matching_policy: MATCHING_POLICY,
      });
    } catch (reason) {
      if (activeVaultRoot.current === actingVaultRoot) {
        setError(errorMessage(reason));
      }
    } finally {
      if (vaultLoadRequestGeneration.current === actingLoadGeneration) {
        setExecutingRunIds((current) => withoutRun(current, runId));
      }
    }
    // Executions persist imports, Review Items and progress as they go, even when they fail.
    try {
      await Promise.all([refreshVaultData(actingVaultRoot), refreshRuns(actingVaultRoot)]);
    } catch (reason) {
      if (activeVaultRoot.current === actingVaultRoot) {
        setError(errorMessage(reason));
      }
    }
  }

  async function applyRunAction(runId: number, action: RunAction) {
    const actingVaultRoot = loadedVaultRoot;
    setBusyRunIds((current) => new Set(current).add(runId));
    setError(null);
    try {
      await invoke<AcquisitionRun>(`${action}_acquisition_run`, { run_id: runId });
      await refreshRuns(actingVaultRoot);
    } catch (reason) {
      if (activeVaultRoot.current === actingVaultRoot) {
        setError(errorMessage(reason));
      }
    } finally {
      setBusyRunIds((current) => withoutRun(current, runId));
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
          onClick={() => setActiveView("library")}
        >
          Library ({entries.length})
        </button>
        <button
          type="button"
          className={activeView === "review" ? "active" : ""}
          onClick={() => setActiveView("review")}
        >
          Review ({reviewItems.length})
        </button>
        <button
          type="button"
          className={activeView === "acquire" ? "active" : ""}
          onClick={() => setActiveView("acquire")}
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

      {activeView === "library" ? <LibraryView entries={entries} /> : null}
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

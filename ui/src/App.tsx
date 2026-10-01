import { invoke } from "@tauri-apps/api/core";
import { FormEvent, useCallback, useRef, useState } from "react";

import { LibraryView } from "./LibraryView";
import { ReviewView } from "./ReviewView";
import { errorMessage } from "./types";
import type { LibraryEntry, ReviewDecision, ReviewItem } from "./types";

export function App() {
  const activeVaultRoot = useRef<string | null>(null);
  const vaultLoadRequestGeneration = useRef(0);
  const reviewMutationGeneration = useRef(0);
  const reviewRefreshRequestGeneration = useRef(0);
  const [vaultRoot, setVaultRoot] = useState(".game-media-vault");
  const [loadedVaultRoot, setLoadedVaultRoot] = useState<string | null>(null);
  const [entries, setEntries] = useState<LibraryEntry[]>([]);
  const [reviewItems, setReviewItems] = useState<ReviewItem[]>([]);
  const [activeView, setActiveView] = useState<"library" | "review">("library");
  const [loading, setLoading] = useState(false);
  const [resolvingIds, setResolvingIds] = useState<Set<number>>(() => new Set());
  const [error, setError] = useState<string | null>(null);
  const releaseCountLabel = `${entries.length} ${entries.length === 1 ? "release" : "releases"}`;
  const reviewCountLabel = `${reviewItems.length} ${reviewItems.length === 1 ? "review" : "reviews"}`;

  async function loadVault(event?: FormEvent) {
    event?.preventDefault();
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
    try {
      // The backend keeps the opened vault; later commands never send a path.
      await invoke("open_vault", { vault_root: requestedVaultRoot, create: false });
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
            invoke<ReviewItem[]>("list_review_items", { vault_root: resolvingVaultRoot }),
            invoke<LibraryEntry[]>("list_library", { vault_root: resolvingVaultRoot }),
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

      <form className="vault-picker" onSubmit={loadVault}>
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
      </nav>

      {activeView === "library" ? (
        <LibraryView entries={entries} />
      ) : (
        <ReviewView
          items={reviewItems}
          resolvingIds={resolvingIds}
          onResolve={resolveReviewItem}
          onLoadPreview={loadReviewPreview}
        />
      )}
    </main>
  );
}

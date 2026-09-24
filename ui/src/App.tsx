import { invoke } from "@tauri-apps/api/core";
import { FormEvent, useRef, useState } from "react";

import { LibraryView } from "./LibraryView";
import { ReviewView } from "./ReviewView";
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
      const [library, reviews] = await Promise.all([
        invoke<LibraryEntry[]>("list_library", { vault_root: requestedVaultRoot }),
        invoke<ReviewItem[]>("list_review_items", { vault_root: requestedVaultRoot }),
      ]);
      if (
        activeVaultRoot.current !== requestedVaultRoot ||
        loadGeneration !== vaultLoadRequestGeneration.current
      ) {
        return;
      }
      setEntries(library);
      if (reviewMutationGeneration.current === reviewGenerationAtLoadStart) {
        setReviewItems(reviews);
      }
      setLoadedVaultRoot(requestedVaultRoot);
    } catch (reason) {
      if (
        activeVaultRoot.current === requestedVaultRoot &&
        loadGeneration === vaultLoadRequestGeneration.current
      ) {
        setError(String(reason));
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
        vault_root: resolvingVaultRoot,
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
      const reviews = await invoke<ReviewItem[]>("list_review_items", {
        vault_root: resolvingVaultRoot,
      });
      if (
        activeVaultRoot.current !== resolvingVaultRoot ||
        resolvingRefreshGeneration !== reviewRefreshRequestGeneration.current
      ) {
        return;
      }
      setReviewItems(reviews);
    } catch (reason) {
      if (activeVaultRoot.current === resolvingVaultRoot) {
        setError(String(reason));
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
        />
      )}
    </main>
  );
}

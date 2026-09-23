import { invoke } from "@tauri-apps/api/core";
import { FormEvent, useState } from "react";

import { LibraryView } from "./LibraryView";
import { ReviewView } from "./ReviewView";
import type { LibraryEntry, ReviewDecision, ReviewItem } from "./types";

export function App() {
  const [vaultRoot, setVaultRoot] = useState(".game-media-vault");
  const [entries, setEntries] = useState<LibraryEntry[]>([]);
  const [reviewItems, setReviewItems] = useState<ReviewItem[]>([]);
  const [activeView, setActiveView] = useState<"library" | "review">("library");
  const [loading, setLoading] = useState(false);
  const [resolvingId, setResolvingId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const releaseCountLabel = `${entries.length} ${entries.length === 1 ? "release" : "releases"}`;
  const reviewCountLabel = `${reviewItems.length} ${reviewItems.length === 1 ? "review" : "reviews"}`;

  async function loadVault(event?: FormEvent) {
    event?.preventDefault();
    setLoading(true);
    setError(null);
    setEntries([]);
    setReviewItems([]);
    try {
      const [library, reviews] = await Promise.all([
        invoke<LibraryEntry[]>("list_library", { vault_root: vaultRoot }),
        invoke<ReviewItem[]>("list_review_items", { vault_root: vaultRoot }),
      ]);
      setEntries(library);
      setReviewItems(reviews);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }

  async function resolveReviewItem(reviewItemId: number, decision: ReviewDecision) {
    setResolvingId(reviewItemId);
    setError(null);
    try {
      const resolved = await invoke<ReviewItem>("resolve_review_item", {
        vault_root: vaultRoot,
        review_item_id: reviewItemId,
        decision,
      });
      setReviewItems((current) =>
        current.map((item) => (item.id === resolved.id ? resolved : item)),
      );
    } catch (reason) {
      setError(String(reason));
    } finally {
      setResolvingId(null);
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
          resolvingId={resolvingId}
          onResolve={resolveReviewItem}
        />
      )}
    </main>
  );
}

import { invoke } from "@tauri-apps/api/core";
import { FormEvent, useState } from "react";

import { LibraryView } from "./LibraryView";
import type { LibraryEntry } from "./types";

export function App() {
  const [vaultRoot, setVaultRoot] = useState(".game-media-vault");
  const [entries, setEntries] = useState<LibraryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function loadLibrary(event?: FormEvent) {
    event?.preventDefault();
    setLoading(true);
    setError(null);
    setEntries([]);
    try {
      const library = await invoke<LibraryEntry[]>("list_library", {
        vault_root: vaultRoot,
      });
      setEntries(library);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
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
          {entries.length} {entries.length === 1 ? "asset" : "assets"}
        </span>
      </header>

      <form className="vault-picker" onSubmit={loadLibrary}>
        <label htmlFor="vault-root">Vault path</label>
        <div className="vault-controls">
          <input
            id="vault-root"
            value={vaultRoot}
            onChange={(event) => setVaultRoot(event.target.value)}
            spellCheck={false}
          />
          <button type="submit" disabled={loading || vaultRoot.trim().length === 0}>
            {loading ? "Loading…" : "Load library"}
          </button>
        </div>
        <p className="hint">Use the same vault path passed to the CLI with --vault.</p>
      </form>

      {error ? <p className="error-message">{error}</p> : null}
      <LibraryView entries={entries} />
    </main>
  );
}

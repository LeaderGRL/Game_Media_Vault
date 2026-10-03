import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { errorMessage } from "./types";
import type { ExportSummary } from "./types";

/** Where the last export went, kept for the next one. */
const EXPORT_FOLDER_KEY = "game-media-vault.export-folder";

interface ExportPanelProps {
  /** Copies the vault's originals to `destination`, a full folder path. */
  onExport: (destination: string) => Promise<ExportSummary>;
}

/**
 * Copies the vault's media to a folder people browse, as
 * `<platform>/<game>/<Asset Type>/<file>`, picked with the desktop's own folder dialog.
 */
export function ExportPanel({ onExport }: ExportPanelProps) {
  const [destination, setDestination] = useState(rememberedFolder);
  const [exporting, setExporting] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function choose() {
    const picked = await open({ directory: true, title: "Export media to" });
    if (typeof picked === "string") {
      setDestination(picked);
    }
  }

  async function exportMedia() {
    const folder = destination.trim();
    setExporting(true);
    setStatus(null);
    setError(null);
    try {
      const summary = await onExport(folder);
      remember(folder);
      setStatus(describe(summary, folder));
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setExporting(false);
    }
  }

  return (
    <div className="library-actions export-panel">
      <label>
        Export folder
        <input
          aria-label="Export folder"
          value={destination}
          placeholder="Choose where your media should be copied"
          onChange={(event) => setDestination(event.target.value)}
        />
      </label>
      <button type="button" disabled={exporting} onClick={() => void choose()}>
        Choose folder…
      </button>
      <button
        type="button"
        disabled={exporting || destination.trim() === ""}
        onClick={() => void exportMedia()}
      >
        {exporting ? "Exporting…" : "Export to folder"}
      </button>
      {status ? <p role="status">{status}</p> : null}
      {error ? (
        <p className="error-message" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}

function describe(summary: ExportSummary, folder: string) {
  const files = summary.exported === 1 ? "1 file" : `${summary.exported} files`;
  const already =
    summary.already_exported === 0
      ? ""
      : `; ${summary.already_exported} ${summary.already_exported === 1 ? "was" : "were"} already there`;
  return `Copied ${files} to ${folder}${already}.`;
}

function rememberedFolder(): string {
  try {
    return localStorage.getItem(EXPORT_FOLDER_KEY) ?? "";
  } catch {
    return "";
  }
}

function remember(folder: string) {
  try {
    localStorage.setItem(EXPORT_FOLDER_KEY, folder);
  } catch {
    // A webview that keeps no storage simply asks again next time.
  }
}

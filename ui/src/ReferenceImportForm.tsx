import { FormEvent, useState } from "react";

import type { ReferenceCatalogKind, ReferenceImportInput } from "./types";

const CATALOG_KINDS: { value: ReferenceCatalogKind; label: string }[] = [
  { value: "no_intro", label: "No-Intro datafile" },
  { value: "redump", label: "Redump datafile" },
  { value: "mame_software_list", label: "MAME software list" },
];

interface ReferenceImportFormProps {
  importing: boolean;
  /** How the last import went, if one ended. */
  status: string | null;
  onImport: (input: ReferenceImportInput) => void;
}

/** Imports a reference catalog file into the opened vault, as the CLI imports one. */
export function ReferenceImportForm({ importing, status, onImport }: ReferenceImportFormProps) {
  const [kind, setKind] = useState<ReferenceCatalogKind>("no_intro");
  const [file, setFile] = useState("");
  const [maxGames, setMaxGames] = useState("5000");
  const [mameVersion, setMameVersion] = useState("");
  const [problem, setProblem] = useState<string | null>(null);

  function submit(event: FormEvent) {
    event.preventDefault();
    const maxGamesValue = Number(maxGames.trim());
    const nextProblem =
      file.trim() === ""
        ? "Name the catalog file to import."
        : !Number.isSafeInteger(maxGamesValue) || maxGamesValue < 1
          ? "Read at least one release, as a whole number."
          : null;
    setProblem(nextProblem);
    if (nextProblem !== null) {
      return;
    }
    const version = mameVersion.trim();
    onImport({
      kind,
      file: file.trim(),
      max_games: maxGamesValue,
      mame_version: kind === "mame_software_list" && version !== "" ? version : null,
    });
  }

  return (
    <form className="reference-import" aria-label="Reference catalog import" onSubmit={submit}>
      <label>
        Catalog
        <select
          value={kind}
          onChange={(event) => setKind(event.target.value as ReferenceCatalogKind)}
        >
          {CATALOG_KINDS.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </label>
      <label>
        Catalog file
        <input value={file} onChange={(event) => setFile(event.target.value)} />
      </label>
      <label>
        Releases to read at most
        <input
          inputMode="numeric"
          value={maxGames}
          onChange={(event) => setMaxGames(event.target.value)}
        />
      </label>
      {kind === "mame_software_list" ? (
        <label>
          MAME version (optional)
          <input value={mameVersion} onChange={(event) => setMameVersion(event.target.value)} />
        </label>
      ) : null}
      <button type="submit" disabled={importing}>
        {importing ? "Importing…" : "Import catalog"}
      </button>
      {problem ? (
        <p className="error-message" role="alert">
          {problem}
        </p>
      ) : null}
      {status ? <p className="hint">{status}</p> : null}
    </form>
  );
}

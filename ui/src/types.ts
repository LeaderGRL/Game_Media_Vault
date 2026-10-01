export type AssetType = "box_front";

export interface AssetProvenance {
  source_id: string;
  source_asset_label: string | null;
  source_location: string;
}

export type ReleaseAssertionField = "title" | "region" | "revision" | "identifier";

export interface ReleaseAssertion {
  source_id: string;
  source_location: string;
  field: ReleaseAssertionField;
  qualifier: string | null;
  value: string;
}

export interface LibraryAsset {
  asset_id: number;
  asset_type: AssetType;
  object_hash: string;
  byte_len: number;
  original_filename: string;
  provenance: AssetProvenance[];
}

export interface LibraryEntry {
  game_id: number;
  game_title: string;
  release_edition_id: number;
  platform: string;
  region: string;
  edition_name: string;
  assertions: ReleaseAssertion[];
  assets: LibraryAsset[];
}

export interface AssetCandidate {
  provider_candidate_id?: string | null;
  game_title: string;
  platform: string;
  region: string;
  edition_name: string;
  asset_type: AssetType;
  source_id: string;
  source_asset_label: string | null;
  source_url: string;
  original_filename: string;
}

export type MatchSignal = "title" | "platform" | "region" | "edition";

export interface MatchEvidence {
  signal: MatchSignal;
  candidate_value: string;
  release_value: string;
  score_delta: number;
}

export interface ReviewMatchCandidate {
  game_id: number;
  release_edition_id: number;
  game_title: string;
  platform: string;
  region: string;
  edition_name: string;
  score: number;
  evidence: MatchEvidence[];
  assertions: ReleaseAssertion[];
}

export type ReviewDecision =
  | { decision: "accept"; release_edition_id: number }
  | { decision: "reject" }
  | { decision: "defer" };

export type ReviewStatus =
  | "pending"
  | "deferred"
  | "accepted"
  | "rejected"
  | "auto_resolved"
  | "superseded";

export interface ReviewItem {
  id: number;
  candidate_identity: string;
  candidate: AssetCandidate;
  competing_matches: ReviewMatchCandidate[];
  decision: ReviewDecision | null;
  status: ReviewStatus;
}

const PREVIEW_MEDIA_TYPES: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  webp: "image/webp",
  gif: "image/gif",
  bmp: "image/bmp",
  avif: "image/avif",
};

const PREVIEW_SIGNATURES: [number[], string][] = [
  [[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], "image/png"],
  [[0xff, 0xd8, 0xff], "image/jpeg"],
  [[0x47, 0x49, 0x46, 0x38], "image/gif"],
  [[0x42, 0x4d], "image/bmp"],
];

/**
 * Media type of a preview: read from the bytes' signature when it is known, so it always
 * matches the bytes, otherwise derived from the candidate's original filename. SVG is never
 * labelled as an image: blob URLs share the app origin and SVG can carry script.
 */
export function previewMediaType(bytes: Uint8Array, filename: string): string {
  const signature = PREVIEW_SIGNATURES.find(([prefix]) =>
    prefix.every((byte, index) => bytes[index] === byte),
  );
  if (signature) {
    return signature[1];
  }
  if (bytes.length >= 12 && ascii(bytes, 0, 4) === "RIFF" && ascii(bytes, 8, 12) === "WEBP") {
    return "image/webp";
  }
  const extension = filename.includes(".") ? filename.split(".").pop()?.toLowerCase() : undefined;
  return (extension && PREVIEW_MEDIA_TYPES[extension]) || "application/octet-stream";
}

function ascii(bytes: Uint8Array, start: number, end: number) {
  return String.fromCharCode(...bytes.subarray(start, end));
}

/** Error returned by every Tauri command. */
export interface CommandError {
  kind:
    | "invalid_request"
    | "not_found"
    | "conflict"
    | "unsupported"
    | "source_failure"
    | "external";
  message: string;
}

/** Human-readable message of a rejected Tauri invocation. */
export function errorMessage(reason: unknown): string {
  if (typeof reason === "object" && reason !== null && "message" in reason) {
    return String((reason as { message: unknown }).message);
  }
  return String(reason);
}

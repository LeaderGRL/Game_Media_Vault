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

/** Value selected for a release field from its assertions, with the claims it rests on. */
export interface CanonicalValue {
  field: ReleaseAssertionField;
  qualifier: string | null;
  value: string;
  /** Share of the asserting sources that agree with the value, in percent. */
  confidence: number;
  contributing: ReleaseAssertion[];
  conflicting: ReleaseAssertion[];
}

export interface LibraryAsset {
  asset_id: number;
  asset_type: AssetType;
  object_hash: string;
  byte_len: number;
  /** Media type read from the original bytes. */
  media_type: string;
  width: number | null;
  height: number | null;
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
  canonical_values: CanonicalValue[];
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
  if (ftypBrands(bytes).some((brand) => brand === "avif" || brand === "avis")) {
    return "image/avif";
  }
  return filenameMediaType(filename);
}

/** Media type suggested by a filename extension, for media whose bytes are not at hand. */
export function filenameMediaType(filename: string): string {
  const extension = filename.includes(".") ? filename.split(".").pop()?.toLowerCase() : undefined;
  return (extension && PREVIEW_MEDIA_TYPES[extension]) || "application/octet-stream";
}

/**
 * Brands of the `ftyp` box ISO-BMFF images start with: the major brand, then the compatible
 * brands up to the declared box size.
 */
function ftypBrands(bytes: Uint8Array): string[] {
  if (bytes.length < 12 || ascii(bytes, 4, 8) !== "ftyp") {
    return [];
  }
  const size = uint32(bytes, 0);
  // Size 1 announces a 64-bit size after the type; size 0 extends the box to the end.
  const headerSize = size === 1 ? 16 : 8;
  const declaredSize =
    size === 1 ? uint32(bytes, 8) * 2 ** 32 + uint32(bytes, 12) : size === 0 ? bytes.length : size;
  const boxSize = Math.min(bytes.length, declaredSize);
  if (boxSize < headerSize + 4) {
    return [];
  }
  // The major brand, a minor version, then the compatible brands.
  const brands = [ascii(bytes, headerSize, headerSize + 4)];
  for (let offset = headerSize + 8; offset + 4 <= boxSize; offset += 4) {
    brands.push(ascii(bytes, offset, offset + 4));
  }
  return brands;
}

function uint32(bytes: Uint8Array, offset: number) {
  return ((bytes[offset] << 24) | (bytes[offset + 1] << 16) | (bytes[offset + 2] << 8) | bytes[offset + 3]) >>> 0;
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

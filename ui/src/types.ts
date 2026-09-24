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
  run_id: number;
  candidate_identity: string;
  candidate: AssetCandidate;
  competing_matches: ReviewMatchCandidate[];
  decision: ReviewDecision | null;
  status: ReviewStatus;
}

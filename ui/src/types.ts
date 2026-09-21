export type SourceKind = "local_import";
export type AssetType = "box_front";

export interface AssetProvenance {
  source_kind: SourceKind;
  source_location: string;
}

export interface LibraryEntry {
  game_id: number;
  game_title: string;
  release_edition_id: number;
  platform: string;
  region: string;
  edition_name: string;
  asset_id: number;
  asset_type: AssetType;
  object_hash: string;
  byte_len: number;
  original_filename: string;
  provenance: AssetProvenance[];
}

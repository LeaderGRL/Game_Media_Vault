/** Acquisition Request model shared with the Rust `AcquisitionRequestDraft`. */

export type SourceSelection = { mode: "auto" } | { mode: "explicit"; values: string[] };

export type GameSelection = { mode: "all" } | { mode: "explicit"; values: string[] };

export type RetentionPolicy = "keep_everything" | "keep_best_per_type";

export interface AcquisitionRequestDraft {
  sources: SourceSelection;
  platforms: string[];
  games: GameSelection;
  regions: string[];
  languages: string[];
  asset_types: string[];
  quality: null;
  retention: RetentionPolicy;
  limits: Record<string, never>;
}

export type AcquisitionRunStatus = "running" | "paused" | "cancelled" | "completed";

export interface AcquisitionRun {
  id: number;
  request: AcquisitionRequestDraft;
  status: AcquisitionRunStatus;
  queued_work: number;
  awaiting_review_work: number;
  completed_work: number;
}

/** Sources the desktop app can currently execute. */
export const KNOWN_SOURCES = [{ value: "libretro-thumbnails", label: "Libretro Thumbnails" }];

export interface AssetTypeOption {
  value: string;
  label: string;
}

export interface AssetTypeFamily extends AssetTypeOption {
  types: AssetTypeOption[];
}

const option = (value: string, label: string): AssetTypeOption => ({ value, label });

/** The hierarchical Asset Type taxonomy of SPEC §5; a family selects all of its types. */
export const ASSET_TYPE_FAMILIES: AssetTypeFamily[] = [
  {
    ...option("packaging", "Packaging"),
    types: [
      option("box_front", "Box Front"),
      option("box_back", "Box Back"),
      option("spine", "Spine"),
      option("inner_cover", "Inner Cover"),
      option("box_texture", "Box Texture"),
      option("box_3d_render", "Box 3D Render"),
      option("box_3d_model", "Box 3D Model"),
      option("slipcover_sleeve", "Slipcover / Sleeve"),
      option("insert", "Insert"),
    ],
  },
  {
    ...option("physical_media", "Physical Media"),
    types: [
      option("cartridge", "Cartridge"),
      option("cartridge_front", "Cartridge Front"),
      option("cartridge_back", "Cartridge Back"),
      option("cartridge_label", "Cartridge Label"),
      option("disc", "Disc"),
      option("disc_front", "Disc Front"),
      option("disc_back", "Disc Back"),
      option("disc_label", "Disc Label"),
      option("pcb", "PCB"),
      option("cassette_tape", "Cassette / Tape"),
      option("floppy_disk", "Floppy Disk"),
    ],
  },
  {
    ...option("documentation", "Documentation"),
    types: [
      option("manual", "Manual"),
      option("manual_page", "Manual Page"),
      option("strategy_guide", "Strategy Guide"),
      option("map", "Map"),
      option("reference_card", "Reference Card"),
      option("registration_card", "Registration Card"),
      option("warranty_safety_insert", "Warranty / Safety Insert"),
    ],
  },
  {
    ...option("digital_media", "Digital Media"),
    types: [
      option("screenshot", "Screenshot"),
      option("title_screen", "Title Screen"),
      option("gameplay_video", "Gameplay Video"),
      option("trailer", "Trailer"),
      option("logo", "Logo"),
      option("icon", "Icon"),
      option("wallpaper_artwork", "Wallpaper / Artwork"),
    ],
  },
  {
    ...option("promotional_and_historical", "Promotional and Historical"),
    types: [
      option("flyer", "Flyer"),
      option("advertisement", "Advertisement"),
      option("poster", "Poster"),
      option("promotional_artwork", "Promotional Artwork"),
      option("press_material", "Press Material"),
      option("magazine_scan", "Magazine Scan"),
    ],
  },
  {
    ...option("hardware_arcade", "Hardware / Arcade"),
    types: [
      option("arcade_cabinet", "Arcade Cabinet"),
      option("control_panel", "Control Panel"),
      option("marquee", "Marquee"),
      option("bezel", "Bezel"),
      option("controller", "Controller"),
      option("accessory", "Accessory"),
    ],
  },
  {
    ...option("other_family", "Other"),
    types: [
      option("soundtrack", "Soundtrack"),
      option("texture", "Texture"),
      option("3d_model", "3D Model"),
      option("other", "Other"),
    ],
  },
];

export interface AcquisitionForm {
  autoSources: boolean;
  sources: string[];
  /** One platform per line. */
  platforms: string;
  /** One game per line; empty targets all games. */
  games: string;
  /** Comma-separated; empty means any region. */
  regions: string;
  /** Comma-separated; empty means any language. */
  languages: string;
  assetTypes: string[];
  retention: RetentionPolicy;
}

export function emptyAcquisitionForm(): AcquisitionForm {
  return {
    autoSources: false,
    sources: [],
    platforms: "",
    games: "",
    regions: "",
    languages: "",
    assetTypes: [],
    retention: "keep_everything",
  };
}

function entries(text: string, separator: string | RegExp): string[] {
  return text
    .split(separator)
    .map((value) => value.trim())
    .filter((value) => value.length > 0);
}

/**
 * Builds the request draft from the form. Validation stays in Rust: the backend rejects drafts
 * without sources, platforms or Asset Types with the shared validator's message.
 */
export function buildAcquisitionRequest(form: AcquisitionForm): AcquisitionRequestDraft {
  const games = entries(form.games, /\r?\n/);
  return {
    sources: form.autoSources ? { mode: "auto" } : { mode: "explicit", values: form.sources },
    platforms: entries(form.platforms, /\r?\n/),
    games: games.length === 0 ? { mode: "all" } : { mode: "explicit", values: games },
    regions: entries(form.regions, ","),
    languages: entries(form.languages, ","),
    asset_types: form.assetTypes,
    quality: null,
    retention: form.retention,
    limits: {},
  };
}

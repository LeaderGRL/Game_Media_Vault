/** Acquisition Request model shared with the Rust `AcquisitionRequestDraft`. */

export type SourceSelection = { mode: "auto" } | { mode: "explicit"; values: string[] };

export type GameSelection = { mode: "all" } | { mode: "explicit"; values: string[] };

export type RetentionPolicy = "keep_everything" | "keep_best_per_type";

/** Optional quality requirements; omitted fields impose nothing (Rust `QualityRequirements`). */
export interface QualityRequirementsDraft {
  min_width?: number;
  min_height?: number;
}

export interface AcquisitionRequestDraft {
  sources: SourceSelection;
  platforms: string[];
  games: GameSelection;
  regions: string[];
  languages: string[];
  asset_types: string[];
  quality: QualityRequirementsDraft | null;
  retention: RetentionPolicy;
  limits: Record<string, never>;
}

/** Which Sources a request would contact and what each acquires (Rust `AcquisitionPlan`). */
export interface AcquisitionPlan {
  sources: { source_id: string; asset_types: string[] }[];
  excluded: { source_id: string; reason: string }[];
  /** For each requested selector, the planned Sources that acquire it. */
  coverage: { selector: string; sources: string[] }[];
}

export type AcquisitionRunStatus = "running" | "paused" | "cancelled" | "completed";

export interface AcquisitionRun {
  id: number;
  request: AcquisitionRequestDraft;
  status: AcquisitionRunStatus;
  queued_work: number;
  awaiting_review_work: number;
  completed_work: number;
  /** Completed work whose original fell short of the quality requirements. */
  below_quality_work: number;
  /** Completed work whose original a retained Asset outranks under Keep Best Per Type. */
  outranked_work: number;
  /** Completed work whose media its Source no longer serves. */
  unavailable_work: number;
}

/** Sources the desktop app can currently execute. */
export const KNOWN_SOURCES = [
  { value: "libretro-thumbnails", label: "Libretro Thumbnails" },
  { value: "launchbox-games-db", label: "LaunchBox Games Database" },
  { value: "steamgriddb", label: "SteamGridDB" },
];

/** A registered Source as planning knows it. */
export interface SourceDescription {
  source_id: string;
  asset_types: string[];
  direct_media_download: boolean;
  /** Whether the Source takes part in acquisitions on this machine. */
  enabled: boolean;
}

/** A failure of a Source an execution recorded. */
export interface SourceFailure {
  /** Recording order across every Source. */
  sequence: number;
  source_id: string;
  run_id: number;
  stage: "discovery" | "download";
  message: string;
  /** Seconds since the Unix epoch. */
  recorded_at: number;
}

/** The failures executions recorded for one Source, the latest first. */
export interface SourceFailureSummary {
  source_id: string;
  failures: number;
  latest: SourceFailure[];
}

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
  /** Minimum width in pixels; empty imposes none. */
  minWidth: string;
  /** Minimum height in pixels; empty imposes none. */
  minHeight: string;
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
    minWidth: "",
    minHeight: "",
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
 * without sources, platforms or Asset Types with the shared validator's message. Only pixel sizes
 * are checked first, with `pixelSizeProblem`, since JSON cannot carry numbers a `u32` rejects.
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
    quality: qualityRequirements(form),
    retention: form.retention,
    limits: {},
  };
}

/** Largest pixel size the vault stores (Rust `u32`). */
const MAX_PIXEL_SIZE = 4_294_967_295;

/**
 * Why a pixel size field cannot be sent, or `null` when both can. Numbers that do not fit a
 * `u32` would reach the backend as `null` (dropping the requirement) or fail to deserialize, so
 * they are refused before the request is built.
 */
export function pixelSizeProblem(form: AcquisitionForm): string | null {
  for (const [label, text] of [
    ["Minimum width", form.minWidth],
    ["Minimum height", form.minHeight],
  ]) {
    const value = text.trim();
    if (value.length > 0 && (!/^\d+$/.test(value) || Number(value) > MAX_PIXEL_SIZE)) {
      return `${label} must be a whole number of pixels up to ${MAX_PIXEL_SIZE}.`;
    }
  }
  return null;
}

function qualityRequirements(form: AcquisitionForm): QualityRequirementsDraft | null {
  const quality: QualityRequirementsDraft = {};
  const minWidth = form.minWidth.trim();
  const minHeight = form.minHeight.trim();
  if (minWidth.length > 0) {
    quality.min_width = Number(minWidth);
  }
  if (minHeight.length > 0) {
    quality.min_height = Number(minHeight);
  }
  return Object.keys(quality).length > 0 ? quality : null;
}

const ASSET_TYPE_LABELS = new Map(
  ASSET_TYPE_FAMILIES.flatMap((family) => [family, ...family.types]).map((option) => [
    option.value,
    option.label,
  ]),
);

/** The name of an Asset Type or family, as the Acquire view labels it. */
export function assetTypeLabel(value: string): string {
  return ASSET_TYPE_LABELS.get(value) ?? value;
}

/** The name of a known Source, or its id. */
export function sourceLabel(sourceId: string): string {
  return KNOWN_SOURCES.find((source) => source.value === sourceId)?.label ?? sourceId;
}

import { type AcquisitionForm, emptyAcquisitionForm } from "./acquisition";

/** A named set of request fields; applying it keeps the fields it does not set. */
export interface RequestPreset {
  name: string;
  form: Partial<AcquisitionForm>;
}

/** Built-in starting points; applied requests are validated like any other. */
export const BUILT_IN_PRESETS: RequestPreset[] = [
  {
    name: "3D Box Builder",
    form: { assetTypes: ["box_front", "box_back", "spine"], retention: "keep_best_per_type" },
  },
  {
    name: "Archival",
    form: {
      assetTypes: ["packaging", "physical_media", "documentation"],
      retention: "keep_everything",
    },
  },
  {
    name: "Frontend Emulator",
    form: {
      assetTypes: ["box_front", "screenshot", "title_screen", "logo"],
      retention: "keep_best_per_type",
    },
  },
  { name: "Manuals Only", form: { assetTypes: ["manual"], retention: "keep_everything" } },
];

/** Where custom presets persist between sessions. */
const STORAGE_KEY = "game-media-vault.request-presets";

/** Applies a built-in preset, keeping the fields it does not set. */
export function applyPreset(form: AcquisitionForm, preset: RequestPreset): AcquisitionForm {
  return { ...form, ...preset.form };
}

/** Applies a saved preset, which stands for a whole request: fields it lacks start empty. */
export function applySavedPreset(preset: RequestPreset): AcquisitionForm {
  return { ...emptyAcquisitionForm(), ...preset.form };
}

/** The custom presets saved in `storage`, or none when it cannot be read or parsed. */
export function loadCustomPresets(storage: Storage): RequestPreset[] {
  try {
    const saved: unknown = JSON.parse(storage.getItem(STORAGE_KEY) ?? "[]");
    return Array.isArray(saved) ? saved.flatMap(readPreset) : [];
  } catch {
    return [];
  }
}

/** Saves `form` under `name`, replacing the custom preset already named so. */
export function saveCustomPreset(storage: Storage, name: string, form: AcquisitionForm) {
  const presets = loadCustomPresets(storage);
  const index = presets.findIndex((preset) => preset.name === name);
  if (index < 0) {
    presets.push({ name, form });
  } else {
    presets[index] = { name, form };
  }
  store(storage, presets);
}

/** Renames a saved preset; a name another saved preset uses is refused. */
export function renameCustomPreset(storage: Storage, name: string, newName: string) {
  const presets = loadCustomPresets(storage);
  if (newName !== name && presets.some((preset) => preset.name === newName)) {
    throw new Error(`A saved preset is already named "${newName}".`);
  }
  store(
    storage,
    loadCustomPresets(storage).map((preset) =>
      preset.name === name ? { ...preset, name: newName } : preset,
    ),
  );
}

export function deleteCustomPreset(storage: Storage, name: string) {
  store(
    storage,
    loadCustomPresets(storage).filter((preset) => preset.name !== name),
  );
}

function store(storage: Storage, presets: RequestPreset[]) {
  storage.setItem(STORAGE_KEY, JSON.stringify(presets));
}

/** The fields of a saved form, each of the type the form needs; any other field is dropped. */
function readForm(value: object): Partial<AcquisitionForm> {
  const form: Partial<AcquisitionForm> = {};
  const saved = value as Record<string, unknown>;
  const text = (key: keyof AcquisitionForm) =>
    typeof saved[key] === "string" ? (saved[key] as string) : undefined;
  const list = (key: keyof AcquisitionForm) =>
    Array.isArray(saved[key]) && (saved[key] as unknown[]).every((item) => typeof item === "string")
      ? (saved[key] as string[])
      : undefined;
  const fields: Partial<AcquisitionForm> = {
    autoSources: typeof saved.autoSources === "boolean" ? saved.autoSources : undefined,
    sources: list("sources"),
    platforms: text("platforms"),
    games: text("games"),
    regions: text("regions"),
    languages: text("languages"),
    assetTypes: list("assetTypes"),
    retention:
      saved.retention === "keep_everything" ||
      saved.retention === "keep_best_per_type" ||
      saved.retention === "keep_best"
        ? saved.retention
        : undefined,
    keptPerType: text("keptPerType"),
    minWidth: text("minWidth"),
    minHeight: text("minHeight"),
  };
  for (const [key, field] of Object.entries(fields)) {
    if (field !== undefined) {
      Object.assign(form, { [key]: field });
    }
  }
  return form;
}

/** The saved preset `value` holds, or none when it has no name or form. */
function readPreset(value: unknown): RequestPreset[] {
  if (typeof value !== "object" || value === null) {
    return [];
  }
  const { name, form } = value as { name?: unknown; form?: unknown };
  if (typeof name !== "string" || typeof form !== "object" || form === null) {
    return [];
  }
  return [{ name, form: readForm(form) }];
}

/** The webview's local storage, or storage kept for this session when it is unavailable. */
export function defaultPresetStorage(): Storage {
  try {
    return window.localStorage;
  } catch {
    const values = new Map<string, string>();
    return {
      get length() {
        return values.size;
      },
      clear: () => values.clear(),
      getItem: (key) => values.get(key) ?? null,
      key: (index) => [...values.keys()][index] ?? null,
      removeItem: (key) => void values.delete(key),
      setItem: (key, value) => void values.set(key, value),
    };
  }
}

/** Whether `name` belongs to a built-in preset, which custom presets cannot replace. */
export function isBuiltInPresetName(name: string): boolean {
  return BUILT_IN_PRESETS.some((preset) => preset.name === name);
}

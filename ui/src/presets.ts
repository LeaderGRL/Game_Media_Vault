import type { AcquisitionForm } from "./acquisition";

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

export function applyPreset(form: AcquisitionForm, preset: RequestPreset): AcquisitionForm {
  return { ...form, ...preset.form };
}

/** The custom presets saved in `storage`, or none when it cannot be read or parsed. */
export function loadCustomPresets(storage: Storage): RequestPreset[] {
  try {
    const saved: unknown = JSON.parse(storage.getItem(STORAGE_KEY) ?? "[]");
    return Array.isArray(saved) ? saved.filter(isPreset) : [];
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

export function renameCustomPreset(storage: Storage, name: string, newName: string) {
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

function isPreset(value: unknown): value is RequestPreset {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as RequestPreset).name === "string" &&
    typeof (value as RequestPreset).form === "object" &&
    (value as RequestPreset).form !== null
  );
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

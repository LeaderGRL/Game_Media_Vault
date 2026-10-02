import { describe, expect, it } from "vitest";

import { buildAcquisitionRequest, emptyAcquisitionForm } from "./acquisition";
import {
  BUILT_IN_PRESETS,
  applyPreset,
  deleteCustomPreset,
  loadCustomPresets,
  renameCustomPreset,
  saveCustomPreset,
} from "./presets";

function memoryStorage(): Storage {
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

describe("request presets", () => {
  it("offers built-ins with distinct Asset Type sets", () => {
    const names = BUILT_IN_PRESETS.map((preset) => preset.name);
    const types = (name: string) =>
      BUILT_IN_PRESETS.find((preset) => preset.name === name)?.form.assetTypes;

    expect(names).toEqual(["3D Box Builder", "Archival", "Frontend Emulator", "Manuals Only"]);
    expect(types("Manuals Only")).toEqual(["manual"]);
    expect(types("3D Box Builder")).toEqual(["box_front", "box_back", "spine"]);
  });

  it("applies a preset over the request without touching what it does not set", () => {
    const form = {
      ...emptyAcquisitionForm(),
      platforms: "Nintendo - Game Boy",
      games: "Tetris (World) (Rev 1)",
      assetTypes: ["screenshot"],
    };

    const applied = applyPreset(form, BUILT_IN_PRESETS[3]);

    expect(applied.assetTypes).toEqual(["manual"]);
    expect(applied.platforms).toBe("Nintendo - Game Boy");
    expect(applied.games).toBe("Tetris (World) (Rev 1)");
    // The applied request still goes through the normal request builder and validation.
    expect(buildAcquisitionRequest(applied).asset_types).toEqual(["manual"]);
  });

  it("saves, renames, updates and deletes custom presets that survive a restart", () => {
    const storage = memoryStorage();
    const form = { ...emptyAcquisitionForm(), assetTypes: ["box_front"], regions: "Japan" };

    saveCustomPreset(storage, "Japanese covers", form);
    renameCustomPreset(storage, "Japanese covers", "JP covers");
    saveCustomPreset(storage, "JP covers", { ...form, regions: "Japan, Asia" });
    saveCustomPreset(storage, "Other", form);
    deleteCustomPreset(storage, "Other");

    // A new session reads the same storage.
    expect(loadCustomPresets(storage)).toEqual([
      { name: "JP covers", form: { ...form, regions: "Japan, Asia" } },
    ]);
  });

  it("reads no custom presets from storage it cannot use or parse", () => {
    const broken = memoryStorage();
    broken.setItem("game-media-vault.request-presets", "not json");
    const unavailable = {
      ...memoryStorage(),
      getItem: () => {
        throw new Error("blocked");
      },
    } as Storage;

    expect(loadCustomPresets(broken)).toEqual([]);
    expect(loadCustomPresets(unavailable)).toEqual([]);
  });
});

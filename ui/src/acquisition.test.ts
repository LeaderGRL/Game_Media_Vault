import { describe, expect, it } from "vitest";

import {
  ASSET_TYPE_FAMILIES,
  buildAcquisitionRequest,
  emptyAcquisitionForm,
  pixelSizeProblem,
} from "./acquisition";

describe("buildAcquisitionRequest", () => {
  it("turns the form into the shared Acquisition Request draft", () => {
    const request = buildAcquisitionRequest({
      ...emptyAcquisitionForm(),
      sources: ["libretro-thumbnails"],
      platforms: " Nintendo - Game Boy \n\n Nintendo - Nintendo Entertainment System ",
      games: "Tetris (World) (Rev 1)\n Super Mario Land (World) ",
      regions: "Europe, Japan ,",
      languages: "",
      assetTypes: ["box_front", "manual"],
      retention: "keep_best_per_type",
    });

    expect(request).toEqual({
      sources: { mode: "explicit", values: ["libretro-thumbnails"] },
      platforms: ["Nintendo - Game Boy", "Nintendo - Nintendo Entertainment System"],
      games: { mode: "explicit", values: ["Tetris (World) (Rev 1)", "Super Mario Land (World)"] },
      regions: ["Europe", "Japan"],
      languages: [],
      asset_types: ["box_front", "manual"],
      quality: null,
      retention: "keep_best_per_type",
      limits: {},
    });
  });

  it("adds the minimum pixel size as quality requirements", () => {
    const base = { ...emptyAcquisitionForm(), assetTypes: ["box_front"] };

    expect(buildAcquisitionRequest({ ...base, minWidth: " 1000 ", minHeight: "" }).quality).toEqual({
      min_width: 1000,
    });
    expect(buildAcquisitionRequest({ ...base, minWidth: "", minHeight: "1600" }).quality).toEqual({
      min_height: 1600,
    });
    expect(buildAcquisitionRequest(base).quality).toBeNull();
  });

  it("explains pixel sizes the vault cannot store", () => {
    const base = emptyAcquisitionForm();

    // Pixel sizes are unsigned 32-bit integers in the vault.
    for (const value of ["abc", "-5", "12.5", "1e3", "4294967296"]) {
      expect(pixelSizeProblem({ ...base, minHeight: value })).toBe(
        "Minimum height must be a whole number of pixels up to 4294967295.",
      );
    }
    expect(pixelSizeProblem({ ...base, minWidth: " 4294967295 ", minHeight: "" })).toBeNull();
  });

  it("targets all games when no game is listed and keeps Auto explicit", () => {
    const request = buildAcquisitionRequest({
      ...emptyAcquisitionForm(),
      autoSources: true,
      platforms: "Windows",
      assetTypes: ["packaging"],
    });

    expect(request.sources).toEqual({ mode: "auto" });
    expect(request.games).toEqual({ mode: "all" });
  });

  it("offers every taxonomy family with its precise asset types", () => {
    const packaging = ASSET_TYPE_FAMILIES.find((family) => family.value === "packaging");

    expect(ASSET_TYPE_FAMILIES).toHaveLength(7);
    expect(packaging?.types.map((type) => type.value)).toContain("box_front");
    expect(packaging?.types.map((type) => type.value)).toContain("box_3d_model");
  });
});

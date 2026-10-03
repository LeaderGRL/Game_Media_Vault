import { describe, expect, it } from "vitest";

import { CONSOLES, LANGUAGES, REGIONS, consoleName, consolesByMaker, languageName } from "./catalog";

describe("catalog", () => {
  it("names a console without its maker", () => {
    expect(consoleName("Nintendo - Super Nintendo Entertainment System")).toBe(
      "Super Nintendo Entertainment System",
    );
    expect(consoleName("Sega - Master System - Mark III")).toBe("Master System - Mark III");
    expect(consoleName("Homebrew Console")).toBe("Homebrew Console");
  });

  it("groups the consoles a game list is published for by maker, in name order", () => {
    const groups = consolesByMaker(CONSOLES);

    const nintendo = groups.find((group) => group.maker === "Nintendo");
    expect(nintendo?.consoles.map((console) => console.name)).toContain(
      "Super Nintendo Entertainment System",
    );
    expect(groups.map((group) => group.maker)).toEqual(
      [...groups.map((group) => group.maker)].sort((a, b) => a.localeCompare(b)),
    );
  });

  it("finds consoles by maker or name, regardless of case", () => {
    const found = consolesByMaker(CONSOLES, "snes super");

    expect(found).toEqual([]);
    expect(
      consolesByMaker(CONSOLES, "super nintendo").flatMap((group) => group.consoles),
    ).toHaveLength(1);
    expect(consolesByMaker(CONSOLES, "SEGA")[0].maker).toBe("Sega");
  });

  it("offers the regions and languages No-Intro names, languages by name", () => {
    expect(REGIONS.map((region) => region.value)).toContain("Europe");
    // Every region No-Intro names a release with, so none is out of reach of a picker.
    for (const region of ["Poland", "Denmark", "Hong Kong", "Taiwan", "United Kingdom"]) {
      expect(REGIONS.map((option) => option.value)).toContain(region);
    }
    expect(LANGUAGES.find((language) => language.value === "Fr")?.label).toBe("French");
    expect(languageName("De")).toBe("German");
    expect(languageName("Xx")).toBe("Xx");
  });
});

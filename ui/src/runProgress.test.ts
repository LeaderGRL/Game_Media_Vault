import { describe, expect, it } from "vitest";

import type { AcquisitionRun } from "./acquisition";
import { describeRunRequest, runProgress, runTitle } from "./runProgress";

const SNES = "Nintendo - Super Nintendo Entertainment System";

function run(change: Partial<AcquisitionRun> = {}): AcquisitionRun {
  return {
    id: 4,
    request: {
      sources: { mode: "auto" },
      platforms: [SNES],
      games: {
        mode: "platform_bound",
        values: ["A (USA)", "B (USA)", "C (USA)", "D (USA)"].map((game) => ({
          game,
          platform: SNES,
        })),
      },
      regions: ["Europe", "World"],
      languages: [],
      asset_types: ["any"],
      quality: null,
      retention: { keep_best: { per_type: 3 } },
      limits: {},
    },
    planned_sources: ["libretro-thumbnails", "screenscraper"],
    status: "running",
    queued_work: 30,
    awaiting_review_work: 5,
    completed_work: 65,
    below_quality_work: 2,
    outranked_work: 3,
    unavailable_work: 10,
    discoveries: [
      { source_id: "libretro-thumbnails", complete: true, discovered_games: 0 },
      { source_id: "screenscraper", complete: false, discovered_games: 2 },
    ],
    ...change,
  };
}

describe("runProgress", () => {
  it("says how far the search and the downloads of a run went", () => {
    const progress = runProgress(run());

    // One Source searched every game, the other half of them.
    expect(progress.searched).toBeCloseTo(0.75);
    expect(progress.searching).toBe(true);
    expect(progress.sourcesDone).toBe(1);
    expect(progress.sources).toBe(2);
    // 65 of the 100 media found are settled.
    expect(progress.settled).toBe(65);
    expect(progress.found).toBe(100);
    expect(progress.acquired).toBe(50);
    expect(progress.notFound).toBe(10);
    expect(progress.toReview).toBe(5);
    expect(progress.skipped).toBe(5);
    expect(progress.downloaded).toBeCloseTo(0.65);
  });

  it("reads a completed run as searched and downloaded", () => {
    const progress = runProgress(
      run({
        status: "completed",
        queued_work: 0,
        discoveries: [
          { source_id: "libretro-thumbnails", complete: true, discovered_games: 0 },
          { source_id: "screenscraper", complete: true, discovered_games: 0 },
        ],
      }),
    );

    expect(progress.searched).toBe(1);
    expect(progress.searching).toBe(false);
  });

  it("reads a run with nothing found yet as not downloaded", () => {
    const progress = runProgress(
      run({ queued_work: 0, awaiting_review_work: 0, completed_work: 0, unavailable_work: 0 }),
    );

    expect(progress.downloaded).toBe(0);
  });
});

describe("runTitle and describeRunRequest", () => {
  it("names a run after its consoles and says what it keeps to", () => {
    expect(runTitle(run())).toBe("Super Nintendo Entertainment System");
    expect(describeRunRequest(run())).toBe("4 games · Europe · Best 3 of each type");
  });
});

import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AcquisitionRun } from "./acquisition";
import { ActivityView } from "./ActivityView";
import type { LatestMedium } from "./types";

const SNES = "Nintendo - Super Nintendo Entertainment System";

function run(change: Partial<AcquisitionRun> = {}): AcquisitionRun {
  return {
    id: 3,
    request: {
      sources: { mode: "auto" },
      platforms: [SNES],
      games: {
        mode: "platform_bound",
        values: ["A", "B"].map((game) => ({ game, platform: SNES })),
      },
      regions: [],
      languages: [],
      asset_types: ["any"],
      quality: null,
      retention: "keep_everything",
      limits: {},
    },
    planned_sources: ["libretro-thumbnails", "screenscraper"],
    status: "running",
    queued_work: 20,
    awaiting_review_work: 2,
    completed_work: 18,
    below_quality_work: 0,
    outranked_work: 0,
    unavailable_work: 3,
    discoveries: [
      { source_id: "libretro-thumbnails", complete: true, discovered_games: 0 },
      { source_id: "screenscraper", complete: false, discovered_games: 1 },
    ],
    ...change,
  };
}

const latest: LatestMedium = {
  release_edition_id: 7,
  game_title: "Super Metroid",
  platform: SNES,
  region: "Europe",
  asset: {
    asset_id: 41,
    asset_type: "box_front",
    object_hash: "cover-hash",
    byte_len: 1000,
    media_type: "image/png",
    width: 640,
    height: 900,
    original_filename: "front.png",
    document: null,
    derived: [],
    provenance: [],
  },
};

function view(props: Partial<Parameters<typeof ActivityView>[0]> = {}) {
  const handlers = {
    onContinue: vi.fn(),
    onPause: vi.fn(),
    onResume: vi.fn(),
    onCancel: vi.fn(),
    onDismiss: vi.fn(),
  };
  render(
    <ActivityView
      runs={[run()]}
      preparing={[]}
      executingRunIds={new Set([3])}
      waitingRunIds={new Set()}
      busyRunIds={new Set()}
      latest={[]}
      objectUrl={(hash) => `object://${hash}`}
      {...handlers}
      {...props}
    />,
  );
  return handlers;
}

describe("ActivityView", () => {
  it("shows how far a download searched and downloaded, with its counts", () => {
    view();

    // 18 of the 38 media to download are done; the 2 awaiting a decision are not downloads.
    const card = screen.getByRole("article", { name: /Super Nintendo Entertainment System/ });
    expect(within(card).getByText("Downloading")).toBeInTheDocument();
    expect(within(card).getByRole("progressbar", { name: "Search" })).toHaveAttribute(
      "aria-valuenow",
      "75",
    );
    expect(within(card).getByRole("progressbar", { name: "Downloads" })).toHaveAttribute(
      "aria-valuenow",
      "47",
    );
    expect(within(card).getByText("15")).toBeInTheDocument();
    expect(within(card).getByText("acquired")).toBeInTheDocument();
    expect(within(card).getByText("not found")).toBeInTheDocument();
  });

  it("keeps both progress bars on a completed download", () => {
    view({
      runs: [
        run({
          status: "completed",
          queued_work: 0,
          discoveries: [
            { source_id: "libretro-thumbnails", complete: true, discovered_games: 0 },
            { source_id: "screenscraper", complete: true, discovered_games: 0 },
          ],
        }),
      ],
      executingRunIds: new Set(),
    });

    expect(screen.getByRole("progressbar", { name: "Search" })).toHaveAttribute(
      "aria-valuenow",
      "100",
    );
    expect(screen.getByText("2 of 2 Sources done")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Downloads" })).toBeInTheDocument();
  });

  it("pauses or cancels a download under way", () => {
    const handlers = view();

    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(handlers.onPause).toHaveBeenCalledWith(3);
    expect(handlers.onCancel).toHaveBeenCalledWith(3);
  });

  it("offers to go on with a download that stopped, as when the app was closed", () => {
    const handlers = view({ executingRunIds: new Set() });

    expect(screen.getByText("Stopped")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Pause" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    expect(handlers.onContinue).toHaveBeenCalledWith(3);
  });

  it("shows downloads waiting their turn, paused ones and consoles being prepared", () => {
    view({
      runs: [run(), run({ id: 4, status: "paused" })],
      executingRunIds: new Set(),
      waitingRunIds: new Set([3]),
      preparing: [{ key: 1, title: "Mega Drive" }],
    });

    expect(screen.getByText("Waiting")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Resume" })).toBeInTheDocument();
    expect(
      screen.getByRole("article", { name: "Mega Drive" }),
    ).toHaveTextContent("Fetching the game list");
  });

  it("keeps a console that could not start with its reason until dismissed", () => {
    const handlers = view({
      preparing: [{ key: 4, title: "Game Boy", failure: "no game matches the regions asked for" }],
    });

    const card = screen.getByRole("article", { name: "Game Boy" });
    expect(card).toHaveTextContent("Could not start");
    expect(card).toHaveTextContent("no game matches the regions asked for");
    fireEvent.click(within(card).getByRole("button", { name: "Dismiss" }));

    expect(handlers.onDismiss).toHaveBeenCalledWith(4);
  });

  it("shows the media arriving", () => {
    view({ latest: [latest] });

    expect(screen.getByRole("img", { name: "Box Front of Super Metroid" })).toHaveAttribute(
      "src",
      "object://cover-hash",
    );
  });

  it("falls back from an arriving medium's thumbnail to its original, then a placeholder", () => {
    const thumbnail = {
      recipe: { transform: "thumbnail" as const, max_edge: 256 },
      object_hash: "thumb-hash",
      byte_len: 100,
      media_type: "image/png",
      width: 180,
      height: 256,
    };
    view({ latest: [{ ...latest, asset: { ...latest.asset, derived: [thumbnail] } }] });
    const name = "Box Front of Super Metroid";
    expect(screen.getByRole("img", { name })).toHaveAttribute("src", "object://thumb-hash");

    fireEvent.error(screen.getByRole("img", { name }));
    expect(screen.getByRole("img", { name })).toHaveAttribute("src", "object://cover-hash");

    fireEvent.error(screen.getByRole("img", { name }));
    expect(screen.getByRole("img", { name })).not.toHaveAttribute("src");
  });

  it("shows an arriving image whose media type is not inspected yet by its file name", () => {
    view({
      latest: [{ ...latest, asset: { ...latest.asset, media_type: "application/octet-stream" } }],
    });

    expect(screen.getByRole("img", { name: "Box Front of Super Metroid" })).toHaveAttribute(
      "src",
      "object://cover-hash",
    );
  });

  it("explains there is nothing yet", () => {
    view({ runs: [], executingRunIds: new Set() });

    expect(screen.getByText("No downloads yet")).toBeInTheDocument();
  });
});

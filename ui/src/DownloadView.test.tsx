import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { SourceDescription } from "./acquisition";
import { DownloadView } from "./DownloadView";

const SNES = "Nintendo - Super Nintendo Entertainment System";
const MEGA_DRIVE = "Sega - Mega Drive - Genesis";

function source(change: Partial<SourceDescription>): SourceDescription {
  return {
    source_id: "libretro-thumbnails",
    asset_types: ["box_front", "screenshot", "title_screen"],
    direct_media_download: true,
    enabled: true,
    credential: "not_needed",
    credential_fields: [],
    rate_limits: null,
    ...change,
  };
}

const SOURCES = [
  source({}),
  source({
    source_id: "screenscraper",
    asset_types: ["box_front", "manual", "gameplay_video"],
    credential: "missing",
  }),
];

function start() {
  fireEvent.click(screen.getByRole("button", { name: "Start download" }));
}

describe("DownloadView", () => {
  it("downloads every media of every game of each console ticked, one request per console", () => {
    const onStart = vi.fn();
    render(<DownloadView sources={SOURCES} onStart={onStart} />);

    fireEvent.click(screen.getByLabelText("Super Nintendo Entertainment System"));
    fireEvent.click(screen.getByLabelText("Mega Drive - Genesis"));
    start();

    const request = {
      sources: { mode: "auto" },
      games: { mode: "all" },
      regions: [],
      languages: [],
      asset_types: ["any"],
      quality: null,
      retention: "keep_everything",
      limits: {},
    };
    expect(onStart).toHaveBeenCalledWith([
      { ...request, platforms: [SNES] },
      { ...request, platforms: [MEGA_DRIVE] },
    ]);
  });

  it("keeps a number of games, some regions and languages, and the best of each type", () => {
    const onStart = vi.fn();
    render(<DownloadView sources={SOURCES} onStart={onStart} />);
    fireEvent.click(screen.getByLabelText("Super Nintendo Entertainment System"));

    fireEvent.click(screen.getByRole("button", { name: "A number per console" }));
    fireEvent.change(screen.getByLabelText("Games per console"), { target: { value: "50" } });
    const regions = screen.getByRole("group", { name: "Regions" });
    fireEvent.click(within(regions).getByRole("button", { name: "Europe" }));
    fireEvent.click(within(regions).getByRole("button", { name: "France" }));
    fireEvent.click(
      within(screen.getByRole("group", { name: "Languages" })).getByRole("button", {
        name: "French",
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Best 3" }));
    start();

    expect(onStart.mock.calls[0][0][0]).toMatchObject({
      platforms: [SNES],
      regions: ["Europe", "France"],
      languages: ["Fr"],
      retention: { keep_best: { per_type: 3 } },
      limits: { max_games: 50 },
    });
  });

  it("keeps a share of each console's games", () => {
    const onStart = vi.fn();
    render(<DownloadView sources={SOURCES} onStart={onStart} />);
    fireEvent.click(screen.getByLabelText("Super Nintendo Entertainment System"));

    fireEvent.click(screen.getByRole("button", { name: "A share of each console" }));
    fireEvent.change(screen.getByLabelText("Share of games (%)"), { target: { value: "25" } });
    start();

    expect(onStart.mock.calls[0][0][0].limits).toEqual({ games_percent: 25 });
  });

  it("finds consoles by name and ticks every console of a maker at once", () => {
    render(<DownloadView sources={SOURCES} onStart={vi.fn()} />);

    fireEvent.change(screen.getByLabelText("Find a console"), { target: { value: "mega" } });

    expect(screen.getByLabelText("Mega Drive - Genesis")).toBeInTheDocument();
    expect(screen.queryByLabelText("Super Nintendo Entertainment System")).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Find a console"), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "Select every Nintendo console" }));

    expect(screen.getByLabelText("Virtual Boy")).toBeChecked();
    expect(screen.getByLabelText("Game Boy")).toBeChecked();
  });

  it("offers each media type an available Source acquires, and says which ones need a key", () => {
    const onStart = vi.fn();
    render(<DownloadView sources={SOURCES} onStart={onStart} />);
    fireEvent.click(screen.getByLabelText("Super Nintendo Entertainment System"));

    fireEvent.click(screen.getByLabelText("Every kind of media"));

    // ScreenScraper alone acquires manuals, and it needs a key this machine lacks.
    expect(screen.getByLabelText("Manual")).toBeDisabled();
    expect(screen.getByLabelText("Screenshot")).toBeEnabled();
    fireEvent.click(screen.getByLabelText("Box Front"));
    fireEvent.click(screen.getByLabelText("Screenshot"));
    start();

    expect(onStart.mock.calls[0][0][0].asset_types).toEqual(["box_front", "screenshot"]);
  });

  it("keeps to the Sources chosen", () => {
    const onStart = vi.fn();
    render(<DownloadView sources={SOURCES} onStart={onStart} />);
    fireEvent.click(screen.getByLabelText("Super Nintendo Entertainment System"));

    fireEvent.click(screen.getByLabelText("Use every available Source"));
    fireEvent.click(screen.getByLabelText("Libretro Thumbnails"));
    start();

    expect(onStart.mock.calls[0][0][0].sources).toEqual({
      mode: "explicit",
      values: ["libretro-thumbnails"],
    });
  });

  it("waits for a console before starting", () => {
    render(<DownloadView sources={SOURCES} onStart={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Start download" })).toBeDisabled();
  });
});

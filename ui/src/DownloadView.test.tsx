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

  it("refuses a number or share of games it would not keep as shown", () => {
    const onStart = vi.fn();
    render(<DownloadView sources={SOURCES} onStart={onStart} />);
    fireEvent.click(screen.getByLabelText("Super Nintendo Entertainment System"));
    const startButton = screen.getByRole("button", { name: "Start download" });

    fireEvent.click(screen.getByRole("button", { name: "A number per console" }));
    fireEvent.change(screen.getByLabelText("Games per console"), { target: { value: "0" } });
    expect(startButton).toBeDisabled();
    expect(screen.getByText("Enter a whole number of games per console, at least 1.")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "A share of each console" }));
    fireEvent.change(screen.getByLabelText("Share of games (%)"), { target: { value: "200" } });
    expect(startButton).toBeDisabled();
    expect(screen.getByText("Enter a share of games from 1 to 100%.")).toBeVisible();

    fireEvent.change(screen.getByLabelText("Share of games (%)"), { target: { value: "100" } });
    start();
    expect(onStart.mock.calls[0][0][0].limits).toEqual({ games_percent: 100 });
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

  it("ticks every console of a maker even while a search shows only some", () => {
    render(<DownloadView sources={SOURCES} onStart={vi.fn()} />);

    fireEvent.change(screen.getByLabelText("Find a console"), { target: { value: "game boy" } });
    fireEvent.click(screen.getByRole("button", { name: "Select every Nintendo console" }));
    fireEvent.change(screen.getByLabelText("Find a console"), { target: { value: "" } });

    expect(screen.getByLabelText("Game Boy")).toBeChecked();
    expect(screen.getByLabelText("Super Nintendo Entertainment System")).toBeChecked();
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

  it("says why a Source or a media type only it acquires cannot take part", () => {
    render(
      <DownloadView
        sources={[
          source({}),
          source({ source_id: "vgmaps", asset_types: ["map"], enabled: false }),
          source({ source_id: "rawg", asset_types: ["wallpaper_artwork"], credential: "unreadable" }),
        ]}
        onStart={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByLabelText("Every kind of media"));
    expect(screen.getByLabelText("Map").closest(".choice-row")).toHaveTextContent("Disabled");
    expect(screen.getByLabelText("Wallpaper / Artwork").closest(".choice-row")).toHaveTextContent(
      "Key store unreadable",
    );
    fireEvent.click(screen.getByLabelText("Use every available Source"));
    expect(screen.getByLabelText("RAWG").closest(".console-option")).toHaveTextContent(
      "Key store unreadable",
    );
  });

  it("waits for a Source that can take part", () => {
    render(
      <DownloadView
        sources={[source({ enabled: false }), source({ source_id: "rawg", credential: "missing" })]}
        onStart={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByLabelText("Super Nintendo Entertainment System"));

    expect(screen.getByRole("button", { name: "Start download" })).toBeDisabled();
    expect(
      screen.getByText("No Source can take part: enable one or store its key in Sources."),
    ).toBeVisible();
  });

  it("waits for the Sources to be read, and says when they cannot be", () => {
    const { rerender } = render(<DownloadView sources={null} onStart={vi.fn()} />);
    fireEvent.click(screen.getByLabelText("Super Nintendo Entertainment System"));

    expect(screen.getByRole("button", { name: "Start download" })).toBeDisabled();
    expect(screen.getByText("Reading the Sources…")).toBeVisible();

    rerender(<DownloadView sources={null} sourcesError="catalog busy" onStart={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Start download" })).toBeDisabled();
    expect(screen.getByText("The Sources could not be read: catalog busy")).toBeVisible();
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

  it("offers the less common regions on request", () => {
    const onStart = vi.fn();
    render(<DownloadView sources={SOURCES} onStart={onStart} />);
    fireEvent.click(screen.getByLabelText("Super Nintendo Entertainment System"));
    const regions = screen.getByRole("group", { name: "Regions" });
    expect(within(regions).queryByRole("button", { name: "Poland" })).not.toBeInTheDocument();

    fireEvent.click(within(regions).getByRole("button", { name: "More regions" }));
    fireEvent.click(within(regions).getByRole("button", { name: "Poland" }));
    start();

    expect(onStart.mock.calls[0][0][0].regions).toEqual(["Poland"]);
  });

  it("offers only the media the Sources ticked acquire", () => {
    const onStart = vi.fn();
    const launchbox = source({ source_id: "launchbox-games-db", asset_types: ["box_front", "manual"] });
    render(<DownloadView sources={[source({}), launchbox]} onStart={onStart} />);
    fireEvent.click(screen.getByLabelText("Super Nintendo Entertainment System"));
    fireEvent.click(screen.getByLabelText("Every kind of media"));
    fireEvent.click(screen.getByLabelText("Box Front"));
    fireEvent.click(screen.getByLabelText("Manual"));

    fireEvent.click(screen.getByLabelText("Use every available Source"));
    fireEvent.click(screen.getByLabelText("Libretro Thumbnails"));

    // Only LaunchBox acquires manuals, so the request keeps to the types Libretro acquires.
    expect(screen.getByLabelText("Manual")).toBeDisabled();
    expect(screen.getByLabelText("Manual")).not.toBeChecked();
    start();
    expect(onStart.mock.calls[0][0][0]).toMatchObject({
      sources: { mode: "explicit", values: ["libretro-thumbnails"] },
      asset_types: ["box_front"],
    });
  });

  it("keeps a download to worldwide releases alone, or to a less common language", () => {
    const onStart = vi.fn();
    render(<DownloadView sources={SOURCES} onStart={onStart} />);
    fireEvent.click(screen.getByLabelText("Super Nintendo Entertainment System"));

    fireEvent.click(
      within(screen.getByRole("group", { name: "Regions" })).getByRole("button", { name: "World" }),
    );
    const languages = screen.getByRole("group", { name: "Languages" });
    fireEvent.click(within(languages).getByRole("button", { name: "More languages" }));
    fireEvent.click(within(languages).getByRole("button", { name: "Turkish" }));
    start();

    expect(onStart.mock.calls[0][0][0]).toMatchObject({ regions: ["World"], languages: ["Tr"] });
  });
});

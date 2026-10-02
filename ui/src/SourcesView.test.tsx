import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { SourcesView } from "./SourcesView";

describe("SourcesView", () => {
  it("describes each Source with what planning knows of it", () => {
    render(
      <SourcesView
        sources={[
          {
            source_id: "libretro-thumbnails",
            asset_types: ["box_front", "screenshot", "title_screen"],
            direct_media_download: true,
          },
          {
            source_id: "index-only",
            asset_types: ["box_front"],
            direct_media_download: false,
          },
        ]}
      />,
    );

    const libretro = screen.getByRole("region", { name: "Libretro Thumbnails" });
    expect(within(libretro).getByText("Box Front, Screenshot, Title Screen")).toBeInTheDocument();
    expect(within(libretro).getByText("Downloads media directly")).toBeInTheDocument();
    // An unknown Source keeps its id, and one that cannot download says so.
    const indexOnly = screen.getByRole("region", { name: "index-only" });
    expect(within(indexOnly).getByText("Cannot download media directly")).toBeInTheDocument();
  });

  it("says why the Sources could not be read", () => {
    render(<SourcesView sources={null} error="registry unavailable" />);

    expect(screen.getByRole("alert")).toHaveTextContent("registry unavailable");
    expect(screen.queryByText("Reading the registered Sources…")).not.toBeInTheDocument();
  });

  it("says when the Sources are still being read", () => {
    render(<SourcesView sources={null} />);

    expect(screen.getByText("Reading the registered Sources…")).toBeInTheDocument();
  });

  const libretro = {
    source_id: "libretro-thumbnails",
    asset_types: ["box_front"],
    direct_media_download: true,
  };
  const launchbox = {
    source_id: "launchbox-games-db",
    asset_types: ["box_back"],
    direct_media_download: true,
  };

  it("summarizes the failures the loaded vault recorded for each Source", () => {
    render(
      <SourcesView
        sources={[libretro, launchbox]}
        failures={[
          {
            source_id: "libretro-thumbnails",
            failures: 4,
            latest: [
              {
                sequence: 9,
                source_id: "libretro-thumbnails",
                run_id: 3,
                stage: "download",
                message: "HTTP 503 after 4 attempts",
                recorded_at: 1_790_000_000,
              },
            ],
          },
        ]}
      />,
    );

    const failing = screen.getByRole("region", { name: "Libretro Thumbnails" });
    expect(within(failing).getByText("4 failures recorded")).toBeInTheDocument();
    expect(
      within(failing).getByText(
        "2026-09-21 14:13 UTC · download · run 3: HTTP 503 after 4 attempts",
      ),
    ).toBeInTheDocument();
    const healthy = screen.getByRole("region", { name: "LaunchBox Games Database" });
    expect(within(healthy).getByText("No failure recorded")).toBeInTheDocument();
  });

  it("shows no failures without a loaded vault", () => {
    render(<SourcesView sources={[libretro]} failures={null} />);

    expect(screen.queryByText("No failure recorded")).not.toBeInTheDocument();
    expect(screen.queryByText(/failures recorded/)).not.toBeInTheDocument();
  });
});

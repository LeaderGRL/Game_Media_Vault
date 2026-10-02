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

  it("says when the Sources are still being read", () => {
    render(<SourcesView sources={null} />);

    expect(screen.getByText("Reading the registered Sources…")).toBeInTheDocument();
  });
});

import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

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
            enabled: true,
          },
          {
            source_id: "index-only",
            asset_types: ["box_front"],
            direct_media_download: false,
            enabled: true,
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
    enabled: true,
  };
  const launchbox = {
    source_id: "launchbox-games-db",
    asset_types: ["box_back"],
    direct_media_download: true,
    enabled: true,
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

  it("says whether each Source takes part in acquisitions on this machine", () => {
    render(<SourcesView sources={[libretro, { ...launchbox, enabled: false }]} />);

    const enabled = within(screen.getByRole("region", { name: "Libretro Thumbnails" })).getByRole(
      "checkbox",
      { name: "Enabled on this machine" },
    );
    expect(enabled).toBeChecked();
    const disabled = screen.getByRole("region", { name: "LaunchBox Games Database" });
    expect(
      within(disabled).getByRole("checkbox", { name: "Enabled on this machine" }),
    ).not.toBeChecked();
    expect(
      within(disabled).getByText("Takes no part in acquisitions on this machine"),
    ).toBeInTheDocument();
    // Without a way to change it, the state is only shown.
    expect(enabled).toBeDisabled();
  });

  it("enables or disables a Source and says when that failed", async () => {
    const onSetEnabled = vi
      .fn<(sourceId: string, enabled: boolean) => Promise<void>>()
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce(new Error("settings are read-only"));
    render(<SourcesView sources={[launchbox]} onSetEnabled={onSetEnabled} />);
    const toggle = screen.getByRole("checkbox", { name: "Enabled on this machine" });

    fireEvent.click(toggle);
    expect(onSetEnabled).toHaveBeenCalledWith("launchbox-games-db", false);
    // One change at a time.
    expect(toggle).toBeDisabled();
    await vi.waitFor(() => expect(toggle).toBeEnabled());

    fireEvent.click(toggle);
    expect(await screen.findByRole("alert")).toHaveTextContent("settings are read-only");
  });
});

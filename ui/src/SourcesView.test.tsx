import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SourcesView } from "./SourcesView";

// Links open in the system browser, through the opener plugin the tests stand in for.
const { openUrl } = vi.hoisted(() => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl }));

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
            credential: "not_needed",
            rate_limits: null,
            credential_fields: [],
          },
          {
            source_id: "index-only",
            asset_types: ["box_front"],
            direct_media_download: false,
            enabled: true,
            credential: "not_needed",
            rate_limits: null,
            credential_fields: [],
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

  it("shows what a Source is known to limit", () => {
    render(
      <SourcesView
        sources={[
          {
            source_id: "thegamesdb",
            asset_types: ["box_front"],
            direct_media_download: true,
            enabled: true,
            credential: "not_needed",
            rate_limits: "Each API key has a monthly allowance of requests.",
            credential_fields: [],
          },
          {
            source_id: "libretro-thumbnails",
            asset_types: ["box_front"],
            direct_media_download: true,
            enabled: true,
            credential: "not_needed",
            rate_limits: null,
            credential_fields: [],
          },
        ]}
      />,
    );

    const limited = screen.getByRole("region", { name: "TheGamesDB" });
    expect(within(limited).getByText("Limits")).toBeInTheDocument();
    expect(
      within(limited).getByText("Each API key has a monthly allowance of requests."),
    ).toBeInTheDocument();
    // A Source whose limits are unknown says nothing of them.
    const unknown = screen.getByRole("region", { name: "Libretro Thumbnails" });
    expect(within(unknown).queryByText("Limits")).not.toBeInTheDocument();
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
    credential: "not_needed" as const,
    rate_limits: null,
    credential_fields: [],
  };
  const launchbox = {
    source_id: "launchbox-games-db",
    asset_types: ["box_back"],
    direct_media_download: true,
    enabled: true,
    credential: "not_needed" as const,
    rate_limits: null,
    credential_fields: [],
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

  /** The one credential of a Source that needs an API key, as this machine stores it or not. */
  const apiKey = (state: "missing" | "stored" | "unreadable") => [
    { id: "api-key", label: "API key", optional: false, state },
  ];

  const steamgriddb = {
    source_id: "steamgriddb",
    asset_types: ["logo", "icon", "wallpaper_artwork"],
    direct_media_download: true,
    enabled: true,
    credential: "missing" as const,
    credential_fields: apiKey("missing"),
    rate_limits: null,
  };

  it("says whether a Source that needs an API key has one, never asking others", () => {
    render(
      <SourcesView
        sources={[
          libretro,
          steamgriddb,
          {
            ...steamgriddb,
            source_id: "stored-source",
            credential: "stored",
            credential_fields: apiKey("stored"),
          },
          {
            ...steamgriddb,
            source_id: "unreadable-source",
            credential: "unreadable",
            credential_fields: apiKey("unreadable"),
          },
        ]}
      />,
    );

    const keyed = screen.getByRole("region", { name: "SteamGridDB" });
    expect(
      within(keyed).getByText("Needs an API key, which this machine does not store"),
    ).toBeInTheDocument();
    const stored = screen.getByRole("region", { name: "stored-source" });
    expect(
      within(stored).getByText("Stored in this machine's credential store"),
    ).toBeInTheDocument();
    const unreadable = screen.getByRole("region", { name: "unreadable-source" });
    expect(
      within(unreadable).getByText("This machine's credential store could not be read"),
    ).toBeInTheDocument();
    const plain = screen.getByRole("region", { name: "Libretro Thumbnails" });
    expect(within(plain).queryByText("API key")).not.toBeInTheDocument();
    // Without a way to store one, no key is asked for.
    expect(screen.queryByLabelText("API key for SteamGridDB")).not.toBeInTheDocument();
  });

  it("stores a key typed in, never showing it, and forgets a stored one", async () => {
    const onSetApiKey = vi
      .fn<(sourceId: string, field: string, key: string) => Promise<void>>()
      .mockResolvedValue(undefined);
    const onClearApiKey = vi
      .fn<(sourceId: string, field: string) => Promise<void>>()
      .mockResolvedValue(undefined);
    const { rerender } = render(
      <SourcesView
        sources={[steamgriddb]}
        onSetApiKey={onSetApiKey}
        onClearApiKey={onClearApiKey}
      />,
    );
    const field = screen.getByLabelText("API key for SteamGridDB");
    expect(field).toHaveAttribute("type", "password");
    expect(screen.getByRole("button", { name: "Store key" })).toBeDisabled();

    fireEvent.change(field, { target: { value: "user-key-123" } });
    fireEvent.click(screen.getByRole("button", { name: "Store key" }));

    expect(onSetApiKey).toHaveBeenCalledWith("steamgriddb", "api-key", "user-key-123");
    await vi.waitFor(() => expect(field).toHaveValue(""));
    rerender(
      <SourcesView
        sources={[{ ...steamgriddb, credential: "stored", credential_fields: apiKey("stored") }]}
        onSetApiKey={onSetApiKey}
        onClearApiKey={onClearApiKey}
      />,
    );
    expect(screen.queryByText("user-key-123")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Forget key" }));
    expect(onClearApiKey).toHaveBeenCalledWith("steamgriddb", "api-key");
  });

  it("keeps a key it failed to store for another try, and says why", async () => {
    const onSetApiKey = vi
      .fn<(sourceId: string, field: string, key: string) => Promise<void>>()
      .mockRejectedValue(new Error("the credential store is locked"));
    render(<SourcesView sources={[steamgriddb]} onSetApiKey={onSetApiKey} />);
    const field = screen.getByLabelText("API key for SteamGridDB");

    fireEvent.change(field, { target: { value: "user-key-123" } });
    fireEvent.click(screen.getByRole("button", { name: "Store key" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("the credential store is locked");
    expect(field).toHaveValue("user-key-123");
  });
  it("asks for each credential of a Source that needs several, by name", () => {
    const onSetApiKey = vi
      .fn<(sourceId: string, field: string, key: string) => Promise<void>>()
      .mockResolvedValue(undefined);
    render(
      <SourcesView
        sources={[
          {
            ...steamgriddb,
            source_id: "account-source",
            credential_fields: [
              { id: "dev-id", label: "Developer id", optional: false, state: "stored" },
              { id: "user-password", label: "Account password", optional: true, state: "missing" },
            ],
          },
        ]}
        onSetApiKey={onSetApiKey}
      />,
    );

    const source = screen.getByRole("region", { name: "account-source" });
    expect(within(source).getByText("Developer id")).toBeInTheDocument();
    expect(within(source).getByText("Optional; this machine stores none")).toBeInTheDocument();
    fireEvent.change(within(source).getByLabelText("Account password for account-source"), {
      target: { value: "pass word" },
    });
    fireEvent.click(within(source).getAllByRole("button", { name: "Store key" })[1]);

    expect(onSetApiKey).toHaveBeenCalledWith("account-source", "user-password", "pass word");
  });
  it("links RAWG's own site, as its terms ask", () => {
    render(
      <SourcesView
        sources={[{ ...steamgriddb, source_id: "rawg", asset_types: ["screenshot"] }]}
      />,
    );

    const source = screen.getByRole("region", { name: "RAWG" });
    const link = within(source).getByRole("link", { name: "RAWG" });
    expect(link).toHaveAttribute("href", "https://rawg.io");
    fireEvent.click(link);
    expect(openUrl).toHaveBeenCalledWith("https://rawg.io");
  });
});

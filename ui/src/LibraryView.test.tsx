import { fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { LibraryView } from "./LibraryView";
import {
  NO_LIBRARY_FILTERS,
  type LibraryAsset,
  type LibraryEntry,
  type LibraryFilters,
} from "./types";

vi.mock("./modelScene", () => ({ showModel: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
const { pickFolder } = vi.hoisted(() => ({ pickFolder: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: pickFolder }));

const entry: LibraryEntry = {
  game_id: 1,
  game_title: "Metal Gear Solid",
  release_edition_id: 2,
  platform: "Sony - PlayStation",
  region: "France",
  edition_name: "Original",
  assertions: [],
  canonical_values: [],
  preferred_assets: [],
  coverage: null,
  packaging_model: null,
  assets: [
    {
      asset_id: 3,
      asset_type: "box_front",
      object_hash: "abc123",
      byte_len: 4096,
      media_type: "image/png",
      width: 1200,
      height: 1600,
      original_filename: "mgs-front.png",
      document: null,
      derived: [],
      provenance: [
        { source_id: "libretro-thumbnails", source_asset_label: null, source_location: "x" },
      ],
    },
  ],
};

const objectUrl = (hash: string) => "gmv-object://localhost/" + hash;

/** The Library as the App shows it: each search becomes the filters shown. */
function Harness({
  onSearch,
  ...props
}: Partial<Parameters<typeof LibraryView>[0]> & { onSearch: (filters: LibraryFilters) => void }) {
  const [filters, setFilters] = useState(NO_LIBRARY_FILTERS);
  return (
    <LibraryView
      entries={[entry]}
      total={1}
      objectUrl={objectUrl}
      filters={filters}
      platforms={["Sony - PlayStation", "Nintendo - Game Boy"]}
      onSearch={(next) => {
        setFilters(next);
        onSearch(next);
      }}
      {...props}
    />
  );
}

function library(props: Partial<Parameters<typeof LibraryView>[0]> = {}) {
  const onSearch = vi.fn();
  render(<Harness onSearch={onSearch} {...props} />);
  return onSearch;
}

describe("LibraryView", () => {
  it("shows each game as its cover, with its console and how many media it has", () => {
    library();

    const card = screen.getByRole("button", { name: /Metal Gear Solid/ });
    expect(within(card).getByRole("img", { name: "Box Front of Metal Gear Solid" })).toHaveAttribute(
      "src",
      "gmv-object://localhost/abc123",
    );
    expect(card).toHaveTextContent("PlayStation · France");
    expect(card).toHaveTextContent("1 medium");
    expect(screen.getByText("1 game")).toBeInTheDocument();
  });

  it("opens a game's media and where they came from, and closes them", () => {
    library();

    fireEvent.click(screen.getByRole("button", { name: /Metal Gear Solid/ }));
    const detail = screen.getByRole("dialog", { name: "Metal Gear Solid" });
    expect(within(detail).getByText("mgs-front.png")).toBeInTheDocument();
    expect(within(detail).getByText(/libretro-thumbnails/)).toBeInTheDocument();

    fireEvent.click(within(detail).getByRole("button", { name: "Close" }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("filters by console, region, Source, media and status picked from lists", () => {
    const onSearch = library();

    fireEvent.click(screen.getByRole("button", { name: "Consoles" }));
    fireEvent.click(screen.getByLabelText("Game Boy"));
    expect(onSearch).toHaveBeenLastCalledWith({
      ...NO_LIBRARY_FILTERS,
      platforms: ["Nintendo - Game Boy"],
    });

    fireEvent.click(screen.getByRole("button", { name: "Regions" }));
    fireEvent.click(screen.getByLabelText("Europe"));
    fireEvent.click(screen.getByRole("button", { name: "Source" }));
    fireEvent.click(screen.getByLabelText("No-Intro"));
    fireEvent.click(screen.getByRole("button", { name: "Media" }));
    fireEvent.click(screen.getByLabelText("Manual"));
    fireEvent.click(screen.getByRole("button", { name: "Status" }));
    fireEvent.click(screen.getByLabelText("Needs review"));

    expect(onSearch).toHaveBeenLastCalledWith({
      ...NO_LIBRARY_FILTERS,
      platforms: ["Nintendo - Game Boy"],
      regions: ["Europe"],
      sources: ["no-intro"],
      assetTypes: ["manual"],
      statuses: ["needs_review"],
    });
  });

  it("searches titles once submitted, and can show games without media", () => {
    const onSearch = library();

    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "mario" } });
    fireEvent.submit(screen.getByRole("search"));
    expect(onSearch).toHaveBeenLastCalledWith({ ...NO_LIBRARY_FILTERS, text: "mario" });

    fireEvent.click(screen.getByLabelText("Include games without media"));
    expect(onSearch).toHaveBeenLastCalledWith({
      ...NO_LIBRARY_FILTERS,
      text: "mario",
      includeWithoutMedia: true,
    });
  });

  it("invites to download media when the library is empty", () => {
    const onDownload = vi.fn();
    library({ entries: [], total: 0, onDownload });

    expect(screen.getByText("Your library is empty")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Download media" }));

    expect(onDownload).toHaveBeenCalled();
  });

  it("offers to show the media that arrived meanwhile", () => {
    const onRefresh = vi.fn();
    library({ newMedia: true, onRefresh });

    fireEvent.click(screen.getByRole("button", { name: "Show them" }));

    expect(onRefresh).toHaveBeenCalled();
  });

  it("exports the library to a folder from a dialog", async () => {
    const onExport = vi.fn().mockResolvedValue({ exported: 3, already_exported: 0 });
    library({ onExport });

    fireEvent.click(screen.getByRole("button", { name: "Export…" }));
    const dialog = screen.getByRole("dialog", { name: "Export media" });
    fireEvent.change(within(dialog).getByLabelText("Export folder"), {
      target: { value: "D:\\Media" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Export to folder" }));

    expect(await within(dialog).findByText(/Copied 3 files/)).toBeInTheDocument();
  });

  it("shows a game by its first image, never by a document", () => {
    const manual: LibraryAsset = {
      ...entry.assets[0],
      asset_id: 7,
      asset_type: "manual",
      object_hash: "manual-pdf",
      media_type: "application/pdf",
      original_filename: "manual.pdf",
      derived: [
        {
          recipe: { transform: "thumbnail", max_edge: 256 },
          object_hash: "manual-page",
          byte_len: 512,
          media_type: "image/png",
          width: 181,
          height: 256,
        },
      ],
    };
    const screenshot: LibraryAsset = {
      ...entry.assets[0],
      asset_id: 8,
      asset_type: "screenshot",
      object_hash: "shot",
      original_filename: "shot.png",
    };
    library({ entries: [{ ...entry, assets: [manual, screenshot] }] });

    expect(screen.getByRole("img", { name: "Screenshot of Metal Gear Solid" })).toBeInTheDocument();
  });
});

import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LibraryView } from "./LibraryView";
import type { AssetType, LibraryEntry } from "./types";

// The 3D scene needs WebGL, which jsdom lacks; the tests drive it instead.
const { showModel } = vi.hoisted(() => ({ showModel: vi.fn() }));
vi.mock("./modelScene", () => ({ showModel }));

const entry: LibraryEntry = {
  game_id: 1,
  game_title: "Metal Gear Solid",
  release_edition_id: 2,
  platform: "PlayStation",
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
      derived: [],
      provenance: [
        {
          source_id: "local_import",
          source_asset_label: null,
          source_location: "C:/covers/mgs-front.png",
        },
      ],
    },
  ],
};

// Thumbnails rendered for the Library, besides one of another size.
const withThumbnail: LibraryEntry = {
  ...entry,
  assets: [
    {
      ...entry.assets[0],
      derived: [128, 256].map((maxEdge) => ({
        recipe: { transform: "thumbnail", max_edge: maxEdge },
        object_hash: "thumb" + maxEdge,
        byte_len: 512,
        media_type: "image/png",
        width: (maxEdge * 3) / 4,
        height: maxEdge,
      })),
    },
  ],
};

const objectUrl = (hash: string) => "gmv-object://localhost/" + hash;

describe("LibraryView", () => {
  it("shows image originals from the vault object store", () => {
    render(<LibraryView entries={[entry]} objectUrl={objectUrl} />);

    const original = screen.getByRole("img", { name: "Box Front of Metal Gear Solid" });
    expect(original).toHaveAttribute("src", "gmv-object://localhost/abc123");
    expect(original).toHaveAttribute("loading", "lazy");
  });

  it("shows the thumbnail rendered from an original instead of the original", () => {
    render(<LibraryView entries={[withThumbnail]} objectUrl={objectUrl} />);

    expect(screen.getByRole("img", { name: "Box Front of Metal Gear Solid" })).toHaveAttribute(
      "src",
      "gmv-object://localhost/thumb256",
    );
  });

  it("shows the original when its thumbnail cannot be shown", () => {
    render(<LibraryView entries={[withThumbnail]} objectUrl={objectUrl} />);

    fireEvent.error(screen.getByRole("img", { name: "Box Front of Metal Gear Solid" }));

    expect(screen.getByRole("img", { name: "Box Front of Metal Gear Solid" })).toHaveAttribute(
      "src",
      "gmv-object://localhost/abc123",
    );
  });

  it("replaces an original the view cannot show with a placeholder", () => {
    render(<LibraryView entries={[entry]} objectUrl={objectUrl} />);

    // Either the vault cannot serve the original or the view cannot decode its format.
    fireEvent.error(screen.getByRole("img", { name: "Box Front of Metal Gear Solid" }));

    expect(screen.queryByRole("img")).not.toBeInTheDocument();
    expect(screen.getByText("Preview unavailable")).toBeInTheDocument();
  });

  it("shows the pixel size of image originals", () => {
    render(<LibraryView entries={[entry]} objectUrl={objectUrl} />);

    expect(screen.getByText("1200 × 1600 px")).toBeInTheDocument();
  });

  it("falls back to the file name for originals whose media is not inspected yet", () => {
    render(
      <LibraryView
        entries={[
          {
            ...entry,
            assets: [
              {
                ...entry.assets[0],
                media_type: "application/octet-stream",
                width: null,
                height: null,
              },
            ],
          },
        ]}
        objectUrl={objectUrl}
      />,
    );

    expect(screen.getByRole("img", { name: "Box Front of Metal Gear Solid" })).toBeInTheDocument();
  });

  it("renders originals as images by their media type, not their file name", () => {
    render(
      <LibraryView
        entries={[
          {
            ...entry,
            assets: [
              {
                ...entry.assets[0],
                original_filename: "cover.png",
                media_type: "application/pdf",
                width: null,
                height: null,
              },
            ],
          },
        ]}
        objectUrl={objectUrl}
      />,
    );

    expect(screen.queryByRole("img")).not.toBeInTheDocument();
    expect(screen.getByText("cover.png")).toBeInTheDocument();
  });

  it("shows canonical values with their confidence, sources and conflicts", () => {
    const claim = (source_id: string, value: string) => ({
      source_id,
      source_location: "C:/catalogs/" + source_id + ".dat",
      field: "revision" as const,
      qualifier: null,
      value,
    });
    render(
      <LibraryView
        objectUrl={objectUrl}
        entries={[
          {
            ...entry,
            canonical_values: [
              {
                field: "revision",
                qualifier: null,
                value: "Rev 1",
                confidence: 50,
                contributing: [claim("no-intro", "Rev 1")],
                conflicting: [claim("redump", "Rev A")],
              },
            ],
          },
        ]}
      />,
    );

    expect(screen.getByText("Revision")).toBeInTheDocument();
    expect(screen.getByText("Rev 1")).toBeInTheDocument();
    expect(screen.getByText("50% · no-intro")).toBeInTheDocument();
    expect(screen.getByText("Conflicts with redump: Rev A")).toBeInTheDocument();
  });

  it("marks the preferred original and explains why the others rank lower", () => {
    const smaller = {
      ...entry.assets[0],
      asset_id: 4,
      object_hash: "def456",
      width: 640,
      height: 900,
      original_filename: "mgs-front-small.png",
    };
    render(
      <LibraryView
        objectUrl={objectUrl}
        entries={[
          {
            ...entry,
            assets: [entry.assets[0], smaller],
            preferred_assets: [
              {
                asset_type: "box_front",
                asset_id: 3,
                outranks: [
                  {
                    asset_id: 4,
                    reason: { reason: "more_pixels", preferred: 1_920_000, other: 576_000 },
                  },
                ],
              },
            ],
          },
        ]}
      />,
    );

    expect(screen.getByText("Preferred")).toBeInTheDocument();
    expect(
      screen.getByText("Fewer pixels than the preferred original (576000 vs 1920000 px)"),
    ).toBeInTheDocument();
  });

  it("shows the coverage status and what each profile misses", () => {
    render(
      <LibraryView
        objectUrl={objectUrl}
        entries={[
          {
            ...entry,
            coverage: {
              packaging_family: "jewel_case",
              status: "partial",
              profiles: [
                {
                  profile: "packaging",
                  required: ["box_front", "box_back", "spine"],
                  missing: ["box_back", "spine"],
                },
                {
                  profile: "physical",
                  required: ["box_front", "box_back", "spine", "disc", "manual"],
                  missing: ["box_back", "spine", "disc", "manual"],
                },
              ],
            },
          },
        ]}
      />,
    );

    expect(screen.getByText("Partial · Jewel case")).toBeInTheDocument();
    expect(screen.getByText("Packaging misses Box Back, Spine")).toBeInTheDocument();
    expect(screen.getByText("Physical misses Box Back, Spine, Disc, Manual")).toBeInTheDocument();
  });

  it("says when a platform's coverage is not evaluated", () => {
    render(<LibraryView objectUrl={objectUrl} entries={[entry]} />);

    expect(screen.getByText("Coverage not evaluated for this platform")).toBeInTheDocument();
  });

  it("groups media by Asset Type and names each type", () => {
    const screenshot = {
      ...entry.assets[0],
      asset_id: 5,
      asset_type: "screenshot" as const,
      object_hash: "snap123",
      original_filename: "mgs-snap.png",
    };
    render(
      <LibraryView
        objectUrl={objectUrl}
        entries={[{ ...entry, assets: [screenshot, entry.assets[0]] }]}
      />,
    );

    const images = screen.getAllByRole("img").map((image) => image.getAttribute("alt"));
    expect(images).toEqual([
      "Box Front of Metal Gear Solid",
      "Screenshot of Metal Gear Solid",
    ]);
  });

  it("orders every stored Asset Type by the taxonomy", () => {
    const asset = (assetId: number, assetType: AssetType) => ({
      ...entry.assets[0],
      asset_id: assetId,
      asset_type: assetType,
      object_hash: "hash" + assetId,
    });
    render(
      <LibraryView
        objectUrl={objectUrl}
        entries={[
          {
            ...entry,
            assets: [asset(5, "logo"), asset(6, "cartridge_front"), asset(7, "box_back")],
          },
        ]}
      />,
    );

    const images = screen.getAllByRole("img").map((image) => image.getAttribute("alt"));
    expect(images).toEqual([
      "Box Back of Metal Gear Solid",
      "Cartridge Front of Metal Gear Solid",
      "Logo of Metal Gear Solid",
    ]);
  });

  it("shows the imported release and Box Front provenance", () => {
    render(<LibraryView entries={[entry]} objectUrl={objectUrl} />);

    expect(screen.getByRole("heading", { name: "Metal Gear Solid" })).toBeInTheDocument();
    expect(screen.getByText("PlayStation · France · Original")).toBeInTheDocument();
    expect(screen.getByText("Box Front")).toBeInTheDocument();
    expect(screen.getByText("mgs-front.png")).toBeInTheDocument();
    expect(screen.getByText("local_import: C:/covers/mgs-front.png")).toBeInTheDocument();
  });

  it("keeps provenance entries distinct when providers share a location", () => {
    render(
      <LibraryView
        objectUrl={objectUrl}
        entries={[
          {
            ...entry,
            assets: [
              {
                ...entry.assets[0],
                provenance: [
                  {
                    source_id: "provider-a",
                    source_asset_label: "Named_Boxarts",
                    source_location: "https://example.invalid/shared.png",
                  },
                  {
                    source_id: "provider-b",
                    source_asset_label: "Box Front",
                    source_location: "https://example.invalid/shared.png",
                  },
                ],
              },
            ],
          },
        ]}
      />,
    );

    expect(
      screen.getByText("provider-a · Named_Boxarts: https://example.invalid/shared.png"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("provider-b · Box Front: https://example.invalid/shared.png"),
    ).toBeInTheDocument();
  });

  it("shows an assetless No-Intro release with its source assertions", () => {
    render(
      <LibraryView
        objectUrl={objectUrl}
        entries={[
          {
            game_id: 10,
            game_title: "Tetris",
            release_edition_id: 11,
            platform: "Nintendo - Game Boy",
            region: "World",
            edition_name: "Rev 1",
            assets: [],
            canonical_values: [],
            preferred_assets: [],
            coverage: null,
            packaging_model: null,
            assertions: [
              {
                source_id: "no-intro",
                source_location: "C:/catalogs/Nintendo - Game Boy.dat",
                field: "revision",
                qualifier: null,
                value: "Rev 1",
              },
              {
                source_id: "no-intro",
                source_location: "C:/catalogs/Nintendo - Game Boy.dat",
                field: "identifier",
                qualifier: "sha1",
                value: "74591CC9504F3BDEBDAE9D9F8F9D7D68A6B4873B",
              },
            ],
          },
        ]}
      />,
    );

    expect(screen.getByRole("heading", { name: "Tetris" })).toBeInTheDocument();
    expect(screen.getByText("Nintendo - Game Boy · World · Rev 1")).toBeInTheDocument();
    expect(screen.getByText("No media assets")).toBeInTheDocument();
    expect(
      screen.getByText(
        "no-intro · revision: Rev 1 · C:/catalogs/Nintendo - Game Boy.dat",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "no-intro · identifier (sha1): 74591CC9504F3BDEBDAE9D9F8F9D7D68A6B4873B · C:/catalogs/Nintendo - Game Boy.dat",
      ),
    ).toBeInTheDocument();
  });
});

describe("LibraryView packaging models", () => {
  const model = {
    recipe: {
      transform: "packaging_model" as const,
      template: "cardboard_box" as const,
      back_hash: "back1",
      spine_hash: "spine1",
    },
    object_hash: "model1",
    byte_len: 4096,
    media_type: "model/gltf-binary",
    width: null,
    height: null,
  };
  const cardboardBox = (missing: string[]) => ({
    packaging_family: "cardboard_box" as const,
    status: "partial" as const,
    profiles: [
      { profile: "packaging" as const, required: ["box_front", "box_back", "spine"], missing },
    ],
  });

  beforeEach(() => {
    showModel.mockReset();
    showModel.mockReturnValue(() => {});
  });

  it("previews the 3D box of a release and lets it be turned", async () => {
    showModel.mockImplementation((_host, _url, events) => {
      events.onReady();
      return () => {};
    });
    render(
      <LibraryView
        objectUrl={objectUrl}
        entries={[{ ...entry, coverage: cardboardBox([]), packaging_model: model }]}
      />,
    );

    expect(screen.getByRole("figure", { name: "3D box of Metal Gear Solid" })).toBeInTheDocument();
    expect(await screen.findByText("Drag to turn the box")).toBeInTheDocument();
    expect(showModel).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      "gmv-object://localhost/model1",
      expect.anything(),
    );
  });

  it("keeps the Library when the 3D preview cannot be shown", async () => {
    showModel.mockImplementation((_host, _url, events) => {
      events.onError(new Error("Error creating WebGL context."));
      return () => {};
    });
    render(
      <LibraryView
        objectUrl={objectUrl}
        entries={[{ ...entry, coverage: cardboardBox([]), packaging_model: model }]}
      />,
    );

    expect(await screen.findByText("3D preview unavailable")).toBeInTheDocument();
    expect(screen.getByText("Metal Gear Solid")).toBeInTheDocument();
  });

  it("explains what a release without a 3D box still needs", () => {
    render(
      <LibraryView
        objectUrl={objectUrl}
        entries={[
          { ...entry, release_edition_id: 1, coverage: cardboardBox(["box_back", "spine"]) },
          { ...entry, release_edition_id: 2, coverage: cardboardBox([]) },
          {
            ...entry,
            release_edition_id: 3,
            coverage: { ...cardboardBox([]), packaging_family: "jewel_case" },
          },
        ]}
      />,
    );

    expect(screen.getByText("3D box needs Box Back, Spine")).toBeInTheDocument();
    expect(screen.getByText("3D box not built yet")).toBeInTheDocument();
    expect(screen.getByText("No 3D template for jewel case packaging yet")).toBeInTheDocument();
    expect(showModel).not.toHaveBeenCalled();
  });
});

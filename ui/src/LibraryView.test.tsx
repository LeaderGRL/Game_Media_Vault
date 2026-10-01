import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { LibraryView } from "./LibraryView";
import type { LibraryEntry } from "./types";

const entry: LibraryEntry = {
  game_id: 1,
  game_title: "Metal Gear Solid",
  release_edition_id: 2,
  platform: "PlayStation",
  region: "France",
  edition_name: "Original",
  assertions: [],
  canonical_values: [],
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

const objectUrl = (hash: string) => "gmv-object://localhost/" + hash;

describe("LibraryView", () => {
  it("shows image originals from the vault object store", () => {
    render(<LibraryView entries={[entry]} objectUrl={objectUrl} />);

    const original = screen.getByRole("img", { name: "Box Front of Metal Gear Solid" });
    expect(original).toHaveAttribute("src", "gmv-object://localhost/abc123");
    expect(original).toHaveAttribute("loading", "lazy");
  });

  it("replaces an original the vault cannot serve with a placeholder", () => {
    render(<LibraryView entries={[entry]} objectUrl={objectUrl} />);

    fireEvent.error(screen.getByRole("img", { name: "Box Front of Metal Gear Solid" }));

    expect(screen.queryByRole("img")).not.toBeInTheDocument();
    expect(screen.getByText("Original unavailable")).toBeInTheDocument();
  });

  it("shows the pixel size of image originals", () => {
    render(<LibraryView entries={[entry]} objectUrl={objectUrl} />);

    expect(screen.getByText("1200 × 1600 px")).toBeInTheDocument();
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

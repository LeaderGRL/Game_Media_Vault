import { render, screen } from "@testing-library/react";
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
  assets: [
    {
      asset_id: 3,
      asset_type: "box_front",
      object_hash: "abc123",
      byte_len: 4096,
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

describe("LibraryView", () => {
  it("shows the imported release and Box Front provenance", () => {
    render(<LibraryView entries={[entry]} />);

    expect(screen.getByRole("heading", { name: "Metal Gear Solid" })).toBeInTheDocument();
    expect(screen.getByText("PlayStation · France · Original")).toBeInTheDocument();
    expect(screen.getByText("Box Front")).toBeInTheDocument();
    expect(screen.getByText("mgs-front.png")).toBeInTheDocument();
    expect(screen.getByText("local_import: C:/covers/mgs-front.png")).toBeInTheDocument();
  });

  it("keeps provenance entries distinct when providers share a location", () => {
    render(
      <LibraryView
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
        entries={[
          {
            game_id: 10,
            game_title: "Tetris",
            release_edition_id: 11,
            platform: "Nintendo - Game Boy",
            region: "World",
            edition_name: "Rev 1",
            assets: [],
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

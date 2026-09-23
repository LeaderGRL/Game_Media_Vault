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
});

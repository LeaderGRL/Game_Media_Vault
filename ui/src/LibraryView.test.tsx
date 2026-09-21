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
      source_kind: "local_import",
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
    expect(screen.getByText("C:/covers/mgs-front.png")).toBeInTheDocument();
  });
});


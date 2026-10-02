import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ReferenceImportForm } from "./ReferenceImportForm";

describe("ReferenceImportForm", () => {
  it("imports the chosen kind of catalog from the typed file, read up to a bound", () => {
    const onImport = vi.fn();
    render(<ReferenceImportForm importing={false} status={null} onImport={onImport} />);

    fireEvent.change(screen.getByLabelText("Catalog"), { target: { value: "redump" } });
    fireEvent.change(screen.getByLabelText("Catalog file"), {
      target: { value: "D:/dats/psx.dat" },
    });
    fireEvent.change(screen.getByLabelText("Releases to read at most"), {
      target: { value: "250" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Import catalog" }));

    expect(onImport).toHaveBeenCalledWith({
      kind: "redump",
      file: "D:/dats/psx.dat",
      max_games: 250,
      mame_version: null,
    });
    // Only software lists come with a MAME release to name.
    expect(screen.queryByLabelText("MAME version (optional)")).not.toBeInTheDocument();
  });

  it("names the MAME release a software list came with", () => {
    const onImport = vi.fn();
    render(<ReferenceImportForm importing={false} status={null} onImport={onImport} />);

    fireEvent.change(screen.getByLabelText("Catalog"), {
      target: { value: "mame_software_list" },
    });
    fireEvent.change(screen.getByLabelText("Catalog file"), { target: { value: "hash/nes.xml" } });
    fireEvent.change(screen.getByLabelText("MAME version (optional)"), {
      target: { value: "0.268" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Import catalog" }));

    expect(onImport).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "mame_software_list", mame_version: "0.268" }),
    );
  });

  it("refuses an import without a file or with no release to read", () => {
    const onImport = vi.fn();
    render(<ReferenceImportForm importing={false} status={null} onImport={onImport} />);

    fireEvent.click(screen.getByRole("button", { name: "Import catalog" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Name the catalog file to import.");

    fireEvent.change(screen.getByLabelText("Catalog file"), { target: { value: "nes.dat" } });
    fireEvent.change(screen.getByLabelText("Releases to read at most"), {
      target: { value: "0" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Import catalog" }));

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Read at least one release, as a whole number.",
    );
    expect(onImport).not.toHaveBeenCalled();
  });

  it("shows an import under way and how the last one went", () => {
    render(
      <ReferenceImportForm
        importing
        status="Imported 3 releases; 2 malformed records skipped."
        onImport={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Importing…" })).toBeDisabled();
    expect(
      screen.getByText("Imported 3 releases; 2 malformed records skipped."),
    ).toBeInTheDocument();
  });
});

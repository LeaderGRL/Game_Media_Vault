import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ExportPanel } from "./ExportPanel";

// The folder picker is the desktop's own dialog, which the tests stand in for.
const { open } = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open }));

describe("ExportPanel", () => {
  beforeEach(() => {
    open.mockReset();
    localStorage.clear();
  });

  it("exports to the folder picked, and says what it copied", async () => {
    open.mockResolvedValue("D:\\Game Media");
    const onExport = vi.fn().mockResolvedValue({ exported: 12, already_exported: 3 });
    render(<ExportPanel onExport={onExport} />);

    fireEvent.click(screen.getByRole("button", { name: "Choose folder…" }));
    expect(await screen.findByDisplayValue("D:\\Game Media")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Export to folder" }));

    expect(
      await screen.findByText("Copied 12 files to D:\\Game Media; 3 were already there."),
    ).toBeInTheDocument();
    expect(onExport).toHaveBeenCalledWith("D:\\Game Media");
  });

  it("remembers the last folder exported to", async () => {
    const onExport = vi.fn().mockResolvedValue({ exported: 1, already_exported: 0 });
    const { unmount } = render(<ExportPanel onExport={onExport} />);
    fireEvent.change(screen.getByLabelText("Export folder"), {
      target: { value: "C:\\Exports" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Export to folder" }));
    await screen.findByText(/Copied 1 file/);
    unmount();

    render(<ExportPanel onExport={onExport} />);

    expect(screen.getByLabelText("Export folder")).toHaveValue("C:\\Exports");
  });

  it("says why an export failed", async () => {
    const onExport = vi
      .fn()
      .mockRejectedValue({ kind: "invalid_request", message: "choose a full folder path" });
    render(<ExportPanel onExport={onExport} />);
    fireEvent.change(screen.getByLabelText("Export folder"), { target: { value: "exports" } });

    fireEvent.click(screen.getByRole("button", { name: "Export to folder" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("choose a full folder path");
  });
});

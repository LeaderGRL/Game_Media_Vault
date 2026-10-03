import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ANY_ASSET_TYPE, assetTypeLabel } from "./acquisition";
import { QuickAcquire } from "./QuickAcquire";

describe("QuickAcquire", () => {
  it("downloads every kind of media of every game of the platform chosen, from every Source", () => {
    const onDownload = vi.fn();
    render(<QuickAcquire starting={false} onDownload={onDownload} />);

    fireEvent.change(screen.getByLabelText("Platform"), {
      target: { value: "Sega - Mega Drive - Genesis" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Download everything" }));

    expect(onDownload).toHaveBeenCalledWith({
      sources: { mode: "auto" },
      platforms: ["Sega - Mega Drive - Genesis"],
      games: { mode: "all" },
      regions: [],
      languages: [],
      // Every type the planned Sources acquire: a Source left out narrows the request.
      asset_types: [ANY_ASSET_TYPE],
      quality: null,
      retention: "keep_everything",
      limits: {},
    });
  });

  it("suggests the platforms a game list is published for", () => {
    render(<QuickAcquire starting={false} onDownload={vi.fn()} />);

    const platform = screen.getByLabelText("Platform");
    const suggestions = document.getElementById(platform.getAttribute("list") ?? "");

    const names = [...(suggestions?.querySelectorAll("option") ?? [])].map(
      (option) => option.value,
    );
    expect(names).toContain("Sega - Mega Drive - Genesis");
    expect(names).toContain("Sony - PlayStation");
    // Digital catalogs hold no boxed games to collect media for.
    expect(names.some((name) => name.includes("(Digital)"))).toBe(false);
  });

  it("waits for a platform before downloading", () => {
    render(<QuickAcquire starting={false} onDownload={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Download everything" })).toBeDisabled();
  });

  it("says a download is starting", () => {
    render(<QuickAcquire starting onDownload={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Starting…" })).toBeDisabled();
  });
});

describe("assetTypeLabel", () => {
  it("names the request for every type", () => {
    expect(assetTypeLabel(ANY_ASSET_TYPE)).toBe("Every type");
  });
});

describe("QuickAcquire media per type", () => {
  it("keeps the best media of each type up to the number asked for", () => {
    const onDownload = vi.fn();
    render(<QuickAcquire starting={false} onDownload={onDownload} />);
    fireEvent.change(screen.getByLabelText("Platform"), {
      target: { value: "Sega - Mega Drive - Genesis" },
    });

    fireEvent.change(screen.getByLabelText("Media per type (empty for all)"), {
      target: { value: "3" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Download everything" }));

    expect(onDownload.mock.calls[0][0].retention).toEqual({ keep_best: { per_type: 3 } });
  });

  it("refuses a number of media per type below one", () => {
    const onDownload = vi.fn();
    render(<QuickAcquire starting={false} onDownload={onDownload} />);
    fireEvent.change(screen.getByLabelText("Platform"), {
      target: { value: "Sega - Mega Drive - Genesis" },
    });

    fireEvent.change(screen.getByLabelText("Media per type (empty for all)"), {
      target: { value: "0" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Download everything" }));

    expect(onDownload).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("Media per type must be a whole number");
  });
});

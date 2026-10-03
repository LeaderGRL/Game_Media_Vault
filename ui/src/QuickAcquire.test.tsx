import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ASSET_TYPE_FAMILIES } from "./acquisition";
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
      asset_types: ASSET_TYPE_FAMILIES.map((family) => family.value),
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

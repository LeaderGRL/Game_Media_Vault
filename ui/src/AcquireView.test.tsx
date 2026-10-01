import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AcquireView } from "./AcquireView";

describe("AcquireView", () => {
  it("submits the request built from the selected sources, platforms, games and asset types", () => {
    const onStart = vi.fn();
    render(<AcquireView starting={false} onStart={onStart} />);

    fireEvent.click(screen.getByLabelText("Libretro Thumbnails"));
    fireEvent.change(screen.getByLabelText("Platforms (one per line)"), {
      target: { value: "Nintendo - Game Boy" },
    });
    fireEvent.change(screen.getByLabelText("Games (one per line, empty for all)"), {
      target: { value: "Tetris (World) (Rev 1)" },
    });
    fireEvent.click(screen.getByLabelText("Box Front"));
    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));

    expect(onStart).toHaveBeenCalledWith({
      sources: { mode: "explicit", values: ["libretro-thumbnails"] },
      platforms: ["Nintendo - Game Boy"],
      games: { mode: "explicit", values: ["Tetris (World) (Rev 1)"] },
      regions: [],
      languages: [],
      asset_types: ["box_front"],
      quality: null,
      retention: "keep_everything",
      limits: {},
    });
  });

  it("submits minimum pixel sizes as quality requirements", () => {
    const onStart = vi.fn();
    render(<AcquireView starting={false} onStart={onStart} />);

    fireEvent.change(screen.getByLabelText("Minimum width (px, empty for any)"), {
      target: { value: "1000" },
    });
    fireEvent.change(screen.getByLabelText("Minimum height (px, empty for any)"), {
      target: { value: "1400" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));

    expect(onStart.mock.calls[0][0].quality).toEqual({ min_width: 1000, min_height: 1400 });
  });

  it("lets a whole asset type family be selected independently of its types", () => {
    const onStart = vi.fn();
    render(<AcquireView starting={false} onStart={onStart} />);

    fireEvent.click(screen.getByLabelText("All Documentation"));
    fireEvent.click(screen.getByLabelText("Auto (any compatible source)"));
    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));

    expect(onStart.mock.calls[0][0].asset_types).toEqual(["documentation"]);
    expect(onStart.mock.calls[0][0].sources).toEqual({ mode: "auto" });
  });

  it("disables the form while a run is being started", () => {
    render(<AcquireView starting onStart={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Starting…" })).toBeDisabled();
  });
});

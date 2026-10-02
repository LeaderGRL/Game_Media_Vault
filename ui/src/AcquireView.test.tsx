import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
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

  it("refuses a pixel size that is not a whole number of pixels", () => {
    const onStart = vi.fn();
    render(<AcquireView starting={false} onStart={onStart} />);

    fireEvent.change(screen.getByLabelText("Minimum width (px, empty for any)"), {
      target: { value: "12.5" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));

    expect(onStart).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Minimum width must be a whole number of pixels",
    );
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

  it("explains which Source acquires each requested type before starting", async () => {
    const checkPlan = vi.fn().mockResolvedValue({
      sources: [{ source_id: "libretro-thumbnails", asset_types: ["box_front"] }],
      excluded: [{ source_id: "screenscraper", reason: "acquires none of the requested asset types" }],
      coverage: [{ selector: "box_front", sources: ["libretro-thumbnails"] }],
    });
    render(<AcquireView starting={false} onStart={vi.fn()} onCheckPlan={checkPlan} />);
    fireEvent.click(screen.getByLabelText("Auto (any compatible source)"));
    fireEvent.click(screen.getByLabelText("Box Front"));

    fireEvent.click(screen.getByRole("button", { name: "Check plan" }));

    expect(await screen.findByText("Box Front: Libretro Thumbnails")).toBeInTheDocument();
    expect(
      screen.getByText("screenscraper left out: acquires none of the requested asset types"),
    ).toBeInTheDocument();
    expect(checkPlan).toHaveBeenCalledWith(
      expect.objectContaining({ sources: { mode: "auto" }, asset_types: ["box_front"] }),
    );
  });

  it("shows why a plan is refused", async () => {
    const checkPlan = vi
      .fn()
      .mockRejectedValue({ kind: "unsupported", message: "no selected source acquires Manual" });
    render(<AcquireView starting={false} onStart={vi.fn()} onCheckPlan={checkPlan} />);

    fireEvent.click(screen.getByRole("button", { name: "Check plan" }));

    expect(await screen.findByText("no selected source acquires Manual")).toBeInTheDocument();
  });

  it("forgets a checked plan once the request changes", async () => {
    const checkPlan = vi.fn().mockResolvedValue({
      sources: [{ source_id: "libretro-thumbnails", asset_types: ["box_front"] }],
      excluded: [],
      coverage: [{ selector: "box_front", sources: ["libretro-thumbnails"] }],
    });
    render(<AcquireView starting={false} onStart={vi.fn()} onCheckPlan={checkPlan} />);
    fireEvent.click(screen.getByRole("button", { name: "Check plan" }));
    expect(await screen.findByText("Box Front: Libretro Thumbnails")).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("Screenshot"));

    await waitFor(() =>
      expect(screen.queryByText("Box Front: Libretro Thumbnails")).not.toBeInTheDocument(),
    );
  });

  it("drops plan checks that end after the request changed", async () => {
    let resolveCheck: (plan: unknown) => void = () => {};
    let rejectCheck: (reason: unknown) => void = () => {};
    const checkPlan = vi
      .fn()
      .mockReturnValueOnce(new Promise((resolve) => (resolveCheck = resolve)))
      .mockReturnValueOnce(new Promise((_, reject) => (rejectCheck = reject)));
    render(<AcquireView starting={false} onStart={vi.fn()} onCheckPlan={checkPlan} />);
    fireEvent.click(screen.getByRole("button", { name: "Check plan" }));
    fireEvent.click(screen.getByLabelText("Screenshot"));
    fireEvent.click(screen.getByRole("button", { name: "Check plan" }));
    fireEvent.click(screen.getByLabelText("Screenshot"));

    await act(async () => {
      resolveCheck({
        sources: [{ source_id: "libretro-thumbnails", asset_types: ["box_front"] }],
        excluded: [],
        coverage: [{ selector: "box_front", sources: ["libretro-thumbnails"] }],
      });
      rejectCheck({ kind: "unsupported", message: "a stale refusal" });
    });

    expect(checkPlan).toHaveBeenCalledTimes(2);
    expect(screen.queryByText("Box Front: Libretro Thumbnails")).not.toBeInTheDocument();
    expect(screen.queryByText("a stale refusal")).not.toBeInTheDocument();
  });
});

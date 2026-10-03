import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AcquireView } from "./AcquireView";

describe("AcquireView", () => {
  beforeEach(() => window.localStorage.clear());

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
      screen.getByText("ScreenScraper left out: acquires none of the requested asset types"),
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

  it("applies a built-in preset and keeps the fields it does not set editable", () => {
    const onStart = vi.fn();
    render(<AcquireView starting={false} onStart={onStart} />);
    fireEvent.change(screen.getByLabelText("Platforms (one per line)"), {
      target: { value: "Nintendo - Game Boy" },
    });

    fireEvent.change(screen.getByLabelText("Preset"), { target: { value: "Manuals Only" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply preset" }));
    fireEvent.click(screen.getByLabelText("Screenshot"));
    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));

    expect(screen.getByLabelText("Manual")).toBeChecked();
    expect(onStart.mock.calls[0][0]).toMatchObject({
      platforms: ["Nintendo - Game Boy"],
      asset_types: ["manual", "screenshot"],
    });
  });

  it("saves, renames and deletes custom presets that survive a restart", () => {
    const { unmount } = render(<AcquireView starting={false} onStart={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Regions (comma-separated, empty for any)"), {
      target: { value: "Japan" },
    });
    fireEvent.change(screen.getByLabelText("Preset name"), { target: { value: "JP" } });
    fireEvent.click(screen.getByRole("button", { name: "Save preset" }));
    unmount();

    render(<AcquireView starting={false} onStart={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Preset"), { target: { value: "JP" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply preset" }));
    expect(screen.getByLabelText("Regions (comma-separated, empty for any)")).toHaveValue("Japan");

    fireEvent.change(screen.getByLabelText("Preset name"), { target: { value: "Japan only" } });
    fireEvent.click(screen.getByRole("button", { name: "Rename preset" }));
    expect(screen.getByRole("option", { name: "Japan only" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Delete preset" }));
    expect(screen.queryByRole("option", { name: "Japan only" })).not.toBeInTheDocument();
  });

  it("keeps built-in presets from being replaced", () => {
    render(<AcquireView starting={false} onStart={vi.fn()} />);

    fireEvent.change(screen.getByLabelText("Preset name"), { target: { value: "Archival" } });
    fireEvent.click(screen.getByRole("button", { name: "Save preset" }));

    expect(screen.getByRole("alert")).toHaveTextContent('"Archival" is a built-in preset.');
    expect(screen.queryByRole("group", { name: "Saved" })).not.toBeInTheDocument();
  });

  it("refuses to rename a saved preset onto another saved preset's name", () => {
    render(<AcquireView starting={false} onStart={vi.fn()} />);
    for (const name of ["JP", "EU"]) {
      fireEvent.change(screen.getByLabelText("Preset name"), { target: { value: name } });
      fireEvent.click(screen.getByRole("button", { name: "Save preset" }));
    }

    fireEvent.change(screen.getByLabelText("Preset"), { target: { value: "JP" } });
    fireEvent.change(screen.getByLabelText("Preset name"), { target: { value: "EU" } });
    fireEvent.click(screen.getByRole("button", { name: "Rename preset" }));

    expect(screen.getByRole("alert")).toHaveTextContent('A saved preset is already named "EU".');
    expect(screen.getAllByRole("option", { name: "EU" })).toHaveLength(1);
    expect(screen.getByRole("option", { name: "JP" })).toBeInTheDocument();
  });

  it("applies a saved preset as a whole request, ignoring fields it cannot use", () => {
    window.localStorage.setItem(
      "game-media-vault.request-presets",
      JSON.stringify([{ name: "Odd", form: { assetTypes: null, regions: "Japan", minWidth: 4 } }]),
    );
    render(<AcquireView starting={false} onStart={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Platforms (one per line)"), {
      target: { value: "Nintendo - Game Boy" },
    });

    fireEvent.change(screen.getByLabelText("Preset"), { target: { value: "Odd" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply preset" }));

    expect(screen.getByLabelText("Regions (comma-separated, empty for any)")).toHaveValue("Japan");
    expect(screen.getByLabelText("Platforms (one per line)")).toHaveValue("");
    expect(screen.getByLabelText("Minimum width (px, empty for any)")).toHaveValue("");
  });

  it("saves a preset when Enter is pressed in its name without starting a run", () => {
    const onStart = vi.fn();
    render(<AcquireView starting={false} onStart={onStart} />);
    const name = screen.getByLabelText("Preset name");

    fireEvent.change(name, { target: { value: "Mine" } });
    const pressed = fireEvent.keyDown(name, { key: "Enter" });

    // The default action, submitting the request form, is prevented.
    expect(pressed).toBe(false);

    expect(onStart).not.toHaveBeenCalled();
    expect(screen.getByRole("option", { name: "Mine" })).toBeInTheDocument();
  });

  it("says when presets cannot be saved and keeps the view usable", () => {
    const storage = {
      ...window.localStorage,
      getItem: () => null,
      setItem: () => {
        throw new Error("quota exceeded");
      },
    } as unknown as Storage;
    render(<AcquireView starting={false} onStart={vi.fn()} presetStorage={storage} />);

    fireEvent.change(screen.getByLabelText("Preset name"), { target: { value: "Mine" } });
    fireEvent.click(screen.getByRole("button", { name: "Save preset" }));

    expect(screen.getByRole("alert")).toHaveTextContent("Presets cannot be saved in this window.");
    expect(screen.queryByRole("option", { name: "Mine" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start acquisition" })).toBeEnabled();
  });

  it("keeps saved presets for the session when local storage is unavailable", () => {
    const unavailable = vi.spyOn(window, "localStorage", "get").mockImplementation(() => {
      throw new Error("storage blocked");
    });
    try {
      render(<AcquireView starting={false} onStart={vi.fn()} />);

      fireEvent.change(screen.getByLabelText("Preset name"), { target: { value: "Mine" } });
      fireEvent.click(screen.getByRole("button", { name: "Save preset" }));

      expect(screen.getByRole("option", { name: "Mine" })).toBeInTheDocument();
    } finally {
      unavailable.mockRestore();
    }
  });
});

describe("AcquireView retention", () => {
  beforeEach(() => window.localStorage.clear());

  it("keeps the number of assets of each type asked for", () => {
    const onStart = vi.fn();
    render(<AcquireView starting={false} onStart={onStart} />);

    fireEvent.change(screen.getByLabelText("Retention policy"), {
      target: { value: "keep_best" },
    });
    fireEvent.change(screen.getByLabelText("Assets kept per type"), { target: { value: "3" } });
    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));

    expect(onStart.mock.calls[0][0].retention).toEqual({ keep_best: { per_type: 3 } });
  });

  it("refuses a number of assets kept per type below one", () => {
    const onStart = vi.fn();
    render(<AcquireView starting={false} onStart={onStart} />);

    fireEvent.change(screen.getByLabelText("Retention policy"), {
      target: { value: "keep_best" },
    });
    fireEvent.change(screen.getByLabelText("Assets kept per type"), { target: { value: "0" } });
    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));

    expect(onStart).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Assets kept per type must be a whole number from 1",
    );
  });
});

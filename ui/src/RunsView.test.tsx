import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AcquisitionRun } from "./acquisition";
import { RunsView } from "./RunsView";

const run: AcquisitionRun = {
  id: 3,
  request: {
    sources: { mode: "explicit", values: ["libretro-thumbnails"] },
    platforms: ["Nintendo - Game Boy"],
    games: { mode: "explicit", values: ["Tetris (World) (Rev 1)"] },
    regions: [],
    languages: [],
    asset_types: ["box_front"],
    quality: null,
    retention: "keep_everything",
    limits: {},
  },
  status: "running",
  queued_work: 2,
  awaiting_review_work: 1,
  completed_work: 4,
};

function renderRuns(
  runs: AcquisitionRun[],
  busyRunIds = new Set<number>(),
  executingRunIds = new Set<number>(),
) {
  const handlers = {
    onExecute: vi.fn(),
    onPause: vi.fn(),
    onResume: vi.fn(),
    onCancel: vi.fn(),
  };
  render(
    <RunsView
      runs={runs}
      busyRunIds={busyRunIds}
      executingRunIds={executingRunIds}
      {...handlers}
    />,
  );
  return handlers;
}

describe("RunsView", () => {
  it("shows each run with its status, request and work counts", () => {
    renderRuns([run]);

    const card = screen.getByRole("article", { name: "Run #3" });
    expect(within(card).getByText("Running")).toBeInTheDocument();
    expect(within(card).getByText("libretro-thumbnails · Nintendo - Game Boy")).toBeInTheDocument();
    expect(within(card).getByText("2 queued · 1 awaiting review · 4 completed")).toBeInTheDocument();
  });

  it("offers the actions allowed by the run status", () => {
    const handlers = renderRuns([run, { ...run, id: 4, status: "paused" }]);

    const running = screen.getByRole("article", { name: "Run #3" });
    fireEvent.click(within(running).getByRole("button", { name: "Execute" }));
    fireEvent.click(within(running).getByRole("button", { name: "Pause" }));
    expect(within(running).queryByRole("button", { name: "Resume" })).not.toBeInTheDocument();

    const paused = screen.getByRole("article", { name: "Run #4" });
    fireEvent.click(within(paused).getByRole("button", { name: "Resume" }));
    fireEvent.click(within(paused).getByRole("button", { name: "Cancel" }));
    expect(within(paused).queryByRole("button", { name: "Execute" })).not.toBeInTheDocument();

    expect(handlers.onExecute).toHaveBeenCalledWith(3);
    expect(handlers.onPause).toHaveBeenCalledWith(3);
    expect(handlers.onResume).toHaveBeenCalledWith(4);
    expect(handlers.onCancel).toHaveBeenCalledWith(4);
  });

  it("disables the actions of a busy run and hides them for finished runs", () => {
    renderRuns([run, { ...run, id: 5, status: "completed" }], new Set([3]));

    const busy = screen.getByRole("article", { name: "Run #3" });
    expect(within(busy).getByRole("button", { name: "Execute" })).toBeDisabled();
    const completed = screen.getByRole("article", { name: "Run #5" });
    expect(within(completed).queryAllByRole("button")).toHaveLength(0);
  });

  it("keeps pause and cancel available while a run executes", () => {
    const handlers = renderRuns([run], new Set(), new Set([3]));

    const executing = screen.getByRole("article", { name: "Run #3" });
    expect(within(executing).getByRole("button", { name: "Executing…" })).toBeDisabled();
    fireEvent.click(within(executing).getByRole("button", { name: "Pause" }));
    fireEvent.click(within(executing).getByRole("button", { name: "Cancel" }));

    expect(handlers.onPause).toHaveBeenCalledWith(3);
    expect(handlers.onCancel).toHaveBeenCalledWith(3);
  });

  it("explains how to start when there is no run", () => {
    renderRuns([]);

    expect(screen.getByRole("heading", { name: "No acquisition runs" })).toBeInTheDocument();
  });
});

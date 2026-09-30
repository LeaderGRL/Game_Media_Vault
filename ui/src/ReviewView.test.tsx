import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ReviewView } from "./ReviewView";
import type { ReviewItem } from "./types";

const item: ReviewItem = {
  id: 17,
  run_id: 7,
  candidate_identity: "connector:review-game",
  candidate: {
    game_title: "Review Game",
    platform: "Nintendo Entertainment System",
    region: "USA",
    edition_name: "Collector",
    asset_type: "box_front",
    source_id: "fixture-provider",
    source_asset_label: "front",
    source_url: "fixture://review/front",
    original_filename: "front.png",
  },
  competing_matches: [
    {
      game_id: 301,
      release_edition_id: 201,
      game_title: "Review Game",
      platform: "Nintendo Entertainment System",
      region: "USA",
      edition_name: "Standard",
      score: 90,
      evidence: [
        {
          signal: "title",
          candidate_value: "Review Game",
          release_value: "Review Game",
          score_delta: 50,
        },
        {
          signal: "edition",
          candidate_value: "Collector",
          release_value: "Standard",
          score_delta: -5,
        },
      ],
      assertions: [
        {
          source_id: "reference-catalog",
          source_location: "fixture://reference/review-game-standard",
          field: "identifier",
          qualifier: "source_record",
          value: "review-game-standard",
        },
      ],
    },
  ],
  decision: null,
  status: "pending",
};

describe("ReviewView", () => {
  it("shows source evidence, competing scores and score explanation", () => {
    render(
      <ReviewView
        items={[item]}
        resolvingIds={new Set()}
        onResolve={vi.fn()}
        onLoadPreview={vi.fn()}
      />,
    );

    expect(screen.getByRole("heading", { name: "Review Game" })).toBeInTheDocument();
    expect(screen.getByText("fixture-provider · front")).toBeInTheDocument();
    expect(screen.getByText("fixture://review/front")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Load preview" })).toBeInTheDocument();
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
    expect(screen.getByText("Standard")).toBeInTheDocument();
    expect(screen.getByText("Score 90")).toBeInTheDocument();
    expect(screen.getByText("Title: +50")).toBeInTheDocument();
    expect(screen.getByText("Edition: -5")).toBeInTheDocument();
    expect(screen.getByText("reference-catalog")).toBeInTheDocument();
    expect(screen.getByText("fixture://reference/review-game-standard")).toBeInTheDocument();
  });

  it("loads previews through the backend instead of using the persisted locator", async () => {
    const createObjectUrl = vi.fn(() => "blob:review-preview");
    const revokeObjectUrl = vi.fn();
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: createObjectUrl,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: revokeObjectUrl,
    });
    const onLoadPreview = vi.fn().mockResolvedValue({
      media_type: "image/png",
      bytes: [137, 80, 78, 71],
    });
    const remoteItem: ReviewItem = {
      ...item,
      candidate: {
        ...item.candidate,
        source_url: "https://example.invalid/review/401",
      },
    };

    const { unmount } = render(
      <ReviewView
        items={[remoteItem]}
        resolvingIds={new Set()}
        onResolve={vi.fn()}
        onLoadPreview={onLoadPreview}
      />,
    );

    expect(onLoadPreview).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Load preview" }));
    expect(onLoadPreview).toHaveBeenCalledWith(17);
    const preview = await screen.findByRole("img", { name: "Review Game box front candidate" });
    expect(preview).toHaveAttribute("src", "blob:review-preview");
    expect(preview).not.toHaveAttribute("src", "https://example.invalid/review/401");
    expect(createObjectUrl).toHaveBeenCalledOnce();

    unmount();
    expect(revokeObjectUrl).toHaveBeenCalledWith("blob:review-preview");
  });

  it("emits accept reject and defer decisions", () => {
    const onResolve = vi.fn();
    render(
      <ReviewView
        items={[item]}
        resolvingIds={new Set()}
        onResolve={onResolve}
        onLoadPreview={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Accept Standard" }));
    expect(onResolve).toHaveBeenLastCalledWith(17, {
      decision: "accept",
      release_edition_id: 201,
    });

    fireEvent.click(screen.getByRole("button", { name: "Reject candidate" }));
    expect(onResolve).toHaveBeenLastCalledWith(17, { decision: "reject" });

    fireEvent.click(screen.getByRole("button", { name: "Defer review" }));
    expect(onResolve).toHaveBeenLastCalledWith(17, { decision: "defer" });
  });

  it("labels reviews closed by matching re-evaluation", () => {
    const onLoadPreview = vi.fn();
    const closedItem: ReviewItem = {
      ...item,
      status: "auto_resolved",
      candidate: {
        ...item.candidate,
      },
    };
    const { rerender } = render(
      <ReviewView
        items={[closedItem]}
        resolvingIds={new Set()}
        onResolve={vi.fn()}
        onLoadPreview={onLoadPreview}
      />,
    );
    expect(screen.getByText("Auto-resolved")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Load preview" })).not.toBeInTheDocument();
    expect(onLoadPreview).not.toHaveBeenCalled();

    rerender(
      <ReviewView
        items={[{ ...item, status: "superseded" }]}
        resolvingIds={new Set()}
        onResolve={vi.fn()}
        onLoadPreview={vi.fn()}
      />,
    );
    expect(screen.getByText("Superseded")).toBeInTheDocument();

    rerender(
      <ReviewView
        items={[{ ...item, status: "processing" }]}
        resolvingIds={new Set()}
        onResolve={vi.fn()}
        onLoadPreview={vi.fn()}
      />,
    );
    expect(screen.getByText("Processing")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reject candidate" })).toBeDisabled();
  });
});

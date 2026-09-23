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
    },
  ],
  decision: null,
};

describe("ReviewView", () => {
  it("shows source evidence, competing scores and score explanation", () => {
    render(<ReviewView items={[item]} resolvingId={null} onResolve={vi.fn()} />);

    expect(screen.getByRole("heading", { name: "Review Game" })).toBeInTheDocument();
    expect(screen.getByText("fixture-provider · front")).toBeInTheDocument();
    expect(screen.getByText("fixture://review/front")).toBeInTheDocument();
    expect(screen.getByText("Standard")).toBeInTheDocument();
    expect(screen.getByText("Score 90")).toBeInTheDocument();
    expect(screen.getByText("Title: +50")).toBeInTheDocument();
    expect(screen.getByText("Edition: -5")).toBeInTheDocument();
  });

  it("emits accept reject and defer decisions", () => {
    const onResolve = vi.fn();
    render(<ReviewView items={[item]} resolvingId={null} onResolve={onResolve} />);

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
});

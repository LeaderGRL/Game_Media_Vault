import { StrictMode } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ReviewView } from "./ReviewView";
import type { ReviewItem } from "./types";

const item: ReviewItem = {
  id: 17,
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
    const createObjectUrl = vi.fn((_blob: Blob) => "blob:review-preview");
    const revokeObjectUrl = vi.fn();
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: createObjectUrl,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: revokeObjectUrl,
    });
    const onLoadPreview = vi.fn().mockResolvedValue(new Uint8Array([137, 80, 78, 71]).buffer);
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
    const previewBlob = createObjectUrl.mock.calls[0][0];
    expect(previewBlob.type).toBe("image/png");
    expect(previewBlob.size).toBe(4);

    unmount();
    expect(revokeObjectUrl).toHaveBeenCalledWith("blob:review-preview");
  });

  it("releases a loaded preview once its review closes", async () => {
    const revokeObjectUrl = vi.fn();
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn(() => "blob:closing-preview"),
    });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: revokeObjectUrl });
    const onLoadPreview = vi.fn().mockResolvedValue({
      media_type: "image/png",
      bytes: [137, 80, 78, 71],
    });
    const props = { resolvingIds: new Set<number>(), onResolve: vi.fn(), onLoadPreview };
    const { rerender } = render(<ReviewView items={[item]} {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Load preview" }));
    await screen.findByRole("img", { name: "Review Game box front candidate" });

    rerender(
      <ReviewView
        items={[{ ...item, status: "rejected", decision: { decision: "reject" } }]}
        {...props}
      />,
    );

    expect(revokeObjectUrl).toHaveBeenCalledWith("blob:closing-preview");
  });

  it("shows a loaded preview under StrictMode", async () => {
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn(() => "blob:strict-preview"),
    });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
    const onLoadPreview = vi.fn().mockResolvedValue({
      media_type: "image/png",
      bytes: [137, 80, 78, 71],
    });
    render(
      <StrictMode>
        <ReviewView
          items={[item]}
          resolvingIds={new Set()}
          onResolve={vi.fn()}
          onLoadPreview={onLoadPreview}
        />
      </StrictMode>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Load preview" }));

    expect(
      await screen.findByRole("img", { name: "Review Game box front candidate" }),
    ).toHaveAttribute("src", "blob:strict-preview");
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
        items={[
          {
            ...item,
            status: "accepted",
            decision: { decision: "accept", release_edition_id: 201 },
          },
        ]}
        resolvingIds={new Set()}
        onResolve={vi.fn()}
        onLoadPreview={vi.fn()}
      />,
    );
    expect(screen.getByText("Accepted · release #201")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reject candidate" })).toBeDisabled();
  });
});

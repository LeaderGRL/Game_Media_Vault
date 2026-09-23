import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LibraryEntry, ReviewItem } from "./types";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

import { App } from "./App";

const entry: LibraryEntry = {
  game_id: 1,
  game_title: "Metal Gear Solid",
  release_edition_id: 2,
  platform: "PlayStation",
  region: "France",
  edition_name: "Original",
  assertions: [],
  assets: [
    {
      asset_id: 3,
      asset_type: "box_front",
      object_hash: "abc123",
      byte_len: 4096,
      original_filename: "mgs-front.png",
      provenance: [
        {
          source_id: "local_import",
          source_asset_label: null,
          source_location: "C:/covers/mgs-front.png",
        },
      ],
    },
  ],
};

const reviewItem: ReviewItem = {
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
      evidence: [],
      assertions: [],
    },
  ],
  decision: null,
  status: "pending",
};

describe("App", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("clears the previous vault entries when loading another vault fails", async () => {
    invokeMock.mockResolvedValueOnce([entry]).mockResolvedValueOnce([]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    expect(screen.getByText("1 release · 0 reviews")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Vault path"), {
      target: { value: "missing-vault" },
    });
    invokeMock.mockRejectedValueOnce(new Error("catalog does not exist"));
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));

    expect(await screen.findByText(/catalog does not exist/)).toBeInTheDocument();
    expect(screen.queryByText("Metal Gear Solid")).not.toBeInTheDocument();
  });

  it("loads review items and persists a decision through Tauri", async () => {
    invokeMock.mockResolvedValueOnce([]).mockResolvedValueOnce([reviewItem]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (1)" }));
    expect(await screen.findByText("Review Game")).toBeInTheDocument();

    invokeMock.mockResolvedValueOnce({
      ...reviewItem,
      decision: { decision: "accept", release_edition_id: 201 },
      status: "accepted",
    });
    fireEvent.click(screen.getByRole("button", { name: "Accept Standard" }));

    expect(invokeMock).toHaveBeenLastCalledWith("resolve_review_item", {
      vault_root: ".game-media-vault",
      review_item_id: 17,
      decision: { decision: "accept", release_edition_id: 201 },
    });
    expect(await screen.findByText("Accepted · release #201")).toBeInTheDocument();
  });

  it("resolves review items against the vault that was actually loaded", async () => {
    invokeMock.mockResolvedValueOnce([]).mockResolvedValueOnce([reviewItem]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (1)" }));
    fireEvent.change(screen.getByLabelText("Vault path"), {
      target: { value: "another-vault" },
    });

    invokeMock.mockResolvedValueOnce({
      ...reviewItem,
      decision: { decision: "accept", release_edition_id: 201 },
      status: "accepted",
    });
    fireEvent.click(screen.getByRole("button", { name: "Accept Standard" }));

    expect(invokeMock).toHaveBeenLastCalledWith("resolve_review_item", {
      vault_root: ".game-media-vault",
      review_item_id: 17,
      decision: { decision: "accept", release_edition_id: 201 },
    });
  });

  it("ignores a review resolution that returns after another vault is loaded", async () => {
    let finishResolution: ((item: ReviewItem) => void) | undefined;
    const otherReviewItem: ReviewItem = {
      ...reviewItem,
      candidate_identity: "connector:other-review-game",
      candidate: {
        ...reviewItem.candidate,
        game_title: "Other Review Game",
      },
    };
    invokeMock.mockResolvedValueOnce([]).mockResolvedValueOnce([reviewItem]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (1)" }));
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<ReviewItem>((resolve) => {
          finishResolution = resolve;
        }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Accept Standard" }));

    fireEvent.change(screen.getByLabelText("Vault path"), {
      target: { value: "other-vault" },
    });
    invokeMock.mockResolvedValueOnce([]).mockResolvedValueOnce([otherReviewItem]);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Other Review Game")).toBeInTheDocument();

    finishResolution?.({
      ...reviewItem,
      decision: { decision: "accept", release_edition_id: 201 },
    });

    await waitFor(() => {
      expect(screen.getByText("Other Review Game")).toBeInTheDocument();
    });
    expect(screen.queryByText("Accepted · release #201")).not.toBeInTheDocument();
  });
});

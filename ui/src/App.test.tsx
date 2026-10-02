import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LibraryEntry, ReviewItem } from "./types";

const { invokeMock, openVaultMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  openVaultMock: vi.fn(),
}));

// Opening the vault session is mocked separately so each test can script the data commands
// in the order the App issues them.
vi.mock("@tauri-apps/api/core", () => ({
  // Mirrors how Tauri addresses custom protocols on Windows.
  convertFileSrc: (path: string, protocol: string) => "http://" + protocol + ".localhost/" + path,
  invoke: (command: string, args?: Record<string, unknown>) =>
    command === "open_vault"
      ? openVaultMock(args)
      : invokeMock(command, ...(args === undefined ? [] : [args])),
}));

import { App, RUN_PROGRESS_REFRESH_MS } from "./App";

const entry: LibraryEntry = {
  game_id: 1,
  game_title: "Metal Gear Solid",
  release_edition_id: 2,
  platform: "PlayStation",
  region: "France",
  edition_name: "Original",
  assertions: [],
  canonical_values: [],
  preferred_assets: [],
  assets: [
    {
      asset_id: 3,
      asset_type: "box_front",
      object_hash: "abc123",
      byte_len: 4096,
      media_type: "image/png",
      width: 1200,
      height: 1600,
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
    // Unscripted library and run refreshes see empty lists.
    invokeMock.mockImplementation((command: string) =>
      Promise.resolve(
        command === "list_library" || command === "list_acquisition_runs" ? [] : undefined,
      ),
    );
    openVaultMock.mockReset();
    openVaultMock.mockResolvedValue(undefined);
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
    openVaultMock.mockRejectedValueOnce({ kind: "external", message: "catalog does not exist" });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));

    expect(await screen.findByText(/catalog does not exist/)).toBeInTheDocument();
    expect(screen.queryByText("Metal Gear Solid")).not.toBeInTheDocument();
  });

  it("opens the typed vault in the backend session before listing it", async () => {
    invokeMock.mockResolvedValueOnce([entry]).mockResolvedValueOnce([]);
    render(<App />);
    fireEvent.change(screen.getByLabelText("Vault path"), {
      target: { value: "D:/vaults/main" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));

    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    expect(openVaultMock).toHaveBeenCalledWith({ vault_root: "D:/vaults/main", create: false });
    expect(invokeMock).toHaveBeenCalledWith("list_library");
    expect(invokeMock).toHaveBeenCalledWith("list_review_items");
    expect(screen.getByRole("img", { name: "Box Front of Metal Gear Solid" })).toHaveAttribute(
      "src",
      "http://gmv-object.localhost/abc123",
    );
  });

  it("shows the message of a structured backend error", async () => {
    invokeMock.mockResolvedValueOnce([]).mockResolvedValueOnce([reviewItem]);
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (1)" }));

    invokeMock
      .mockRejectedValueOnce({
        kind: "conflict",
        message: "review item #17 cannot be resolved while Rejected",
      })
      // The refused decision refreshes the reviews.
      .mockResolvedValueOnce([reviewItem]);
    fireEvent.click(screen.getByRole("button", { name: "Reject candidate" }));

    expect(
      await screen.findByText("review item #17 cannot be resolved while Rejected"),
    ).toBeInTheDocument();
    expect(screen.queryByText("[object Object]")).not.toBeInTheDocument();
  });

  it("loads review items and persists a decision through Tauri", async () => {
    invokeMock.mockResolvedValueOnce([]).mockResolvedValueOnce([reviewItem]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (1)" }));
    expect(await screen.findByText("Review Game")).toBeInTheDocument();

    const acceptedReviewItem: ReviewItem = {
      ...reviewItem,
      decision: { decision: "accept", release_edition_id: 201 },
      status: "accepted",
    };
    invokeMock.mockResolvedValueOnce(acceptedReviewItem).mockResolvedValueOnce([acceptedReviewItem]);
    fireEvent.click(screen.getByRole("button", { name: "Accept Standard" }));

    expect(invokeMock).toHaveBeenCalledWith("resolve_review_item", {
      review_item_id: 17,
      decision: { decision: "accept", release_edition_id: 201 },
    });
    expect(await screen.findByText("Accepted · release #201")).toBeInTheDocument();
  });

  it("refreshes the library after a review decision moves assets", async () => {
    invokeMock.mockResolvedValueOnce([entry]).mockResolvedValueOnce([reviewItem]);
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (1)" }));

    const rejectedReviewItem: ReviewItem = {
      ...reviewItem,
      decision: { decision: "reject" },
      status: "rejected",
    };
    invokeMock.mockImplementation((command: string) =>
      Promise.resolve(
        command === "resolve_review_item"
          ? rejectedReviewItem
          : command === "list_review_items"
            ? [rejectedReviewItem]
            : [],
      ),
    );
    fireEvent.click(screen.getByRole("button", { name: "Reject candidate" }));

    expect(await screen.findByRole("button", { name: "Library (0)" })).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("list_library");
  });

  it("loads review previews from the vault backend", async () => {
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn(() => "blob:app-review-preview"),
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: vi.fn(),
    });
    const remoteReviewItem: ReviewItem = {
      ...reviewItem,
      candidate: {
        ...reviewItem.candidate,
        source_url: "https://example.invalid/review/401",
      },
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_library" || command === "list_acquisition_runs") {
        return Promise.resolve([]);
      }
      if (command === "list_review_items") {
        return Promise.resolve([remoteReviewItem]);
      }
      if (command === "load_review_preview") {
        return Promise.resolve(new Uint8Array([137, 80, 78, 71]).buffer);
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (1)" }));

    expect(invokeMock).not.toHaveBeenCalledWith("load_review_preview", expect.anything());
    fireEvent.click(screen.getByRole("button", { name: "Load preview" }));
    expect(invokeMock).toHaveBeenCalledWith("load_review_preview", {
      review_item_id: 17,
    });
    const preview = await screen.findByRole("img", { name: "Review Game box front candidate" });
    expect(preview).toHaveAttribute("src", "blob:app-review-preview");
    expect(preview).not.toHaveAttribute("src", "https://example.invalid/review/401");
  });

  it("refreshes a review that changed elsewhere when its decision is refused", async () => {
    const autoResolved: ReviewItem = { ...reviewItem, status: "auto_resolved" };
    let refused = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "resolve_review_item") {
        refused = true;
        return Promise.reject("review item #17 cannot be resolved while AutoResolved");
      }
      if (command === "list_review_items") {
        return Promise.resolve([refused ? autoResolved : reviewItem]);
      }
      if (command === "list_library") {
        // The item was auto-linked elsewhere, so the library changed too.
        return Promise.resolve(refused ? [entry] : []);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (1)" }));

    fireEvent.click(screen.getByRole("button", { name: "Reject candidate" }));

    expect(
      await screen.findByText("review item #17 cannot be resolved while AutoResolved"),
    ).toBeInTheDocument();
    expect(await screen.findByText("Auto-resolved")).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Library (1)" })).toBeInTheDocument();
  });

  it("refreshes the whole review list after a decision", async () => {
    const otherReviewItem: ReviewItem = {
      ...reviewItem,
      id: 18,
      candidate_identity: "candidate:other-review",
    };
    const rejectedReviewItem: ReviewItem = {
      ...reviewItem,
      decision: { decision: "reject" },
      status: "rejected",
    };
    const autoResolvedOther: ReviewItem = {
      ...otherReviewItem,
      status: "auto_resolved",
    };
    invokeMock.mockResolvedValueOnce([]).mockResolvedValueOnce([reviewItem, otherReviewItem]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (2)" }));
    invokeMock.mockResolvedValueOnce(rejectedReviewItem).mockResolvedValueOnce([
      rejectedReviewItem,
      autoResolvedOther,
    ]);
    fireEvent.click(screen.getAllByRole("button", { name: "Reject candidate" })[0]);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("list_review_items");
    });
    expect(await screen.findByText("Rejected")).toBeInTheDocument();
    expect(await screen.findByText("Auto-resolved")).toBeInTheDocument();
  });

  it("resolves review items against the vault that was actually loaded", async () => {
    invokeMock.mockResolvedValueOnce([]).mockResolvedValueOnce([reviewItem]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (1)" }));
    fireEvent.change(screen.getByLabelText("Vault path"), {
      target: { value: "another-vault" },
    });

    const acceptedReviewItem: ReviewItem = {
      ...reviewItem,
      decision: { decision: "accept", release_edition_id: 201 },
      status: "accepted",
    };
    invokeMock.mockResolvedValueOnce(acceptedReviewItem).mockResolvedValueOnce([acceptedReviewItem]);
    fireEvent.click(screen.getByRole("button", { name: "Accept Standard" }));

    expect(invokeMock).toHaveBeenCalledWith("resolve_review_item", {
      review_item_id: 17,
      decision: { decision: "accept", release_edition_id: 201 },
    });
    expect(openVaultMock).toHaveBeenCalledTimes(1);
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

  it("keeps every in-flight review action disabled until its own request settles", async () => {
    let finishFirst: ((item: ReviewItem) => void) | undefined;
    let finishSecond: ((item: ReviewItem) => void) | undefined;
    const secondReviewItem: ReviewItem = {
      ...reviewItem,
      id: 18,
      candidate_identity: "connector:second-review-game",
      candidate: {
        ...reviewItem.candidate,
        game_title: "Second Review Game",
      },
      competing_matches: [
        {
          ...reviewItem.competing_matches[0],
          release_edition_id: 202,
          edition_name: "Deluxe",
        },
      ],
    };
    invokeMock.mockResolvedValueOnce([]).mockResolvedValueOnce([reviewItem, secondReviewItem]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (2)" }));
    invokeMock
      .mockImplementationOnce(
        () =>
          new Promise<ReviewItem>((resolve) => {
            finishFirst = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise<ReviewItem>((resolve) => {
            finishSecond = resolve;
          }),
      );

    const firstAccept = screen.getByRole("button", { name: "Accept Standard" });
    const secondAccept = screen.getByRole("button", { name: "Accept Deluxe" });
    fireEvent.click(firstAccept);
    fireEvent.click(secondAccept);

    expect(firstAccept).toBeDisabled();
    expect(secondAccept).toBeDisabled();

    finishFirst?.({
      ...reviewItem,
      decision: { decision: "accept", release_edition_id: 201 },
      status: "accepted",
    });
    invokeMock.mockResolvedValueOnce([
      {
        ...reviewItem,
        decision: { decision: "accept", release_edition_id: 201 },
        status: "accepted",
      },
      secondReviewItem,
    ]);
    await waitFor(() => expect(firstAccept).toBeDisabled());
    expect(secondAccept).toBeDisabled();

    invokeMock.mockResolvedValueOnce([
      {
        ...reviewItem,
        decision: { decision: "accept", release_edition_id: 201 },
        status: "accepted",
      },
      {
        ...secondReviewItem,
        decision: { decision: "accept", release_edition_id: 202 },
        status: "accepted",
      },
    ]);
    finishSecond?.({
      ...secondReviewItem,
      decision: { decision: "accept", release_edition_id: 202 },
      status: "accepted",
    });
    expect(await screen.findByText("Accepted · release #202")).toBeInTheDocument();
  });

  it("ignores stale review refresh responses from older concurrent resolutions", async () => {
    let finishFirstResolution: ((item: ReviewItem) => void) | undefined;
    let finishSecondResolution: ((item: ReviewItem) => void) | undefined;
    let finishFirstRefresh: ((items: ReviewItem[]) => void) | undefined;
    let refreshCount = 0;
    const secondReviewItem: ReviewItem = {
      ...reviewItem,
      id: 18,
      candidate_identity: "connector:second-review-game",
      candidate: {
        ...reviewItem.candidate,
        game_title: "Second Review Game",
      },
      competing_matches: [
        {
          ...reviewItem.competing_matches[0],
          release_edition_id: 202,
          edition_name: "Deluxe",
        },
      ],
    };
    const acceptedFirst: ReviewItem = {
      ...reviewItem,
      decision: { decision: "accept", release_edition_id: 201 },
      status: "accepted",
    };
    const acceptedSecond: ReviewItem = {
      ...secondReviewItem,
      decision: { decision: "accept", release_edition_id: 202 },
      status: "accepted",
    };
    invokeMock.mockResolvedValueOnce([]).mockResolvedValueOnce([reviewItem, secondReviewItem]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (2)" }));
    invokeMock.mockImplementation((command, args) => {
      if (command === "resolve_review_item") {
        if (args?.review_item_id === 17) {
          return new Promise<ReviewItem>((resolve) => {
            finishFirstResolution = resolve;
          });
        }
        if (args?.review_item_id === 18) {
          return new Promise<ReviewItem>((resolve) => {
            finishSecondResolution = resolve;
          });
        }
      }
      if (command === "list_review_items") {
        refreshCount += 1;
        if (refreshCount === 1) {
          return new Promise<ReviewItem[]>((resolve) => {
            finishFirstRefresh = resolve;
          });
        }
        return Promise.resolve([acceptedFirst, acceptedSecond]);
      }
      if (command === "list_library" || command === "list_acquisition_runs") {
        return Promise.resolve([]);
      }
      throw new Error(`unexpected command: ${command}`);
    });

    fireEvent.click(screen.getByRole("button", { name: "Accept Standard" }));
    fireEvent.click(screen.getByRole("button", { name: "Accept Deluxe" }));

    finishFirstResolution?.(acceptedFirst);
    await waitFor(() => expect(refreshCount).toBe(1));
    finishSecondResolution?.(acceptedSecond);
    expect(await screen.findByText("Accepted · release #202")).toBeInTheDocument();
    expect(screen.getByText("Accepted · release #201")).toBeInTheDocument();

    finishFirstRefresh?.([acceptedFirst, secondReviewItem]);

    await waitFor(() => {
      expect(screen.getByText("Accepted · release #202")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Accept Deluxe" })).toBeDisabled();
    });
  });

  it("does not let an older stale review refresh overwrite a newer resolved item when the newer refresh fails", async () => {
    let finishFirstResolution: ((item: ReviewItem) => void) | undefined;
    let finishSecondResolution: ((item: ReviewItem) => void) | undefined;
    let finishFirstRefresh: ((items: ReviewItem[]) => void) | undefined;
    let refreshCount = 0;
    const secondReviewItem: ReviewItem = {
      ...reviewItem,
      id: 18,
      candidate_identity: "connector:second-review-game",
      candidate: {
        ...reviewItem.candidate,
        game_title: "Second Review Game",
      },
      competing_matches: [
        {
          ...reviewItem.competing_matches[0],
          release_edition_id: 202,
          edition_name: "Deluxe",
        },
      ],
    };
    const acceptedFirst: ReviewItem = {
      ...reviewItem,
      decision: { decision: "accept", release_edition_id: 201 },
      status: "accepted",
    };
    const acceptedSecond: ReviewItem = {
      ...secondReviewItem,
      decision: { decision: "accept", release_edition_id: 202 },
      status: "accepted",
    };
    invokeMock.mockResolvedValueOnce([]).mockResolvedValueOnce([reviewItem, secondReviewItem]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (2)" }));
    invokeMock.mockImplementation((command, args) => {
      if (command === "resolve_review_item") {
        if (args?.review_item_id === 17) {
          return new Promise<ReviewItem>((resolve) => {
            finishFirstResolution = resolve;
          });
        }
        if (args?.review_item_id === 18) {
          return new Promise<ReviewItem>((resolve) => {
            finishSecondResolution = resolve;
          });
        }
      }
      if (command === "list_review_items") {
        refreshCount += 1;
        if (refreshCount === 1) {
          return new Promise<ReviewItem[]>((resolve) => {
            finishFirstRefresh = resolve;
          });
        }
        return Promise.reject(new Error("newer refresh failed"));
      }
      if (command === "list_library" || command === "list_acquisition_runs") {
        return Promise.resolve([]);
      }
      throw new Error(`unexpected command: ${command}`);
    });

    fireEvent.click(screen.getByRole("button", { name: "Accept Standard" }));
    fireEvent.click(screen.getByRole("button", { name: "Accept Deluxe" }));

    finishFirstResolution?.(acceptedFirst);
    await waitFor(() => expect(refreshCount).toBe(1));
    finishSecondResolution?.(acceptedSecond);
    await screen.findByText("newer refresh failed");

    finishFirstRefresh?.([acceptedFirst, secondReviewItem]);

    await waitFor(() => {
      expect(screen.getByText("Accepted · release #201")).toBeInTheDocument();
      expect(screen.getByText("Accepted · release #202")).toBeInTheDocument();
    });
  });

  it("refreshes a completed resolution after reloading the same vault", async () => {
    let finishResolution: ((item: ReviewItem) => void) | undefined;
    const acceptedReviewItem: ReviewItem = {
      ...reviewItem,
      decision: { decision: "accept", release_edition_id: 201 },
      status: "accepted",
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

    invokeMock
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([reviewItem])
      .mockResolvedValueOnce([acceptedReviewItem]);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled();
    });

    finishResolution?.(acceptedReviewItem);

    expect(await screen.findByText("Accepted · release #201")).toBeInTheDocument();
  });

  it("does not let a late same-vault reload overwrite a completed resolution", async () => {
    let finishResolution: ((item: ReviewItem) => void) | undefined;
    let finishReloadReviews: ((items: ReviewItem[]) => void) | undefined;
    const acceptedReviewItem: ReviewItem = {
      ...reviewItem,
      decision: { decision: "accept", release_edition_id: 201 },
      status: "accepted",
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

    invokeMock.mockResolvedValueOnce([]).mockImplementationOnce(
      () =>
        new Promise<ReviewItem[]>((resolve) => {
          finishReloadReviews = resolve;
        }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));

    finishResolution?.(acceptedReviewItem);
    invokeMock.mockResolvedValueOnce([acceptedReviewItem]);
    expect(await screen.findByText("Accepted · release #201")).toBeInTheDocument();

    finishReloadReviews?.([reviewItem]);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled();
    });
    await waitFor(() => {
      expect(screen.getByText("Accepted · release #201")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Accept Standard" })).toBeDisabled();
    });
  });

  it("does not let a late same-vault reload restore the library a decision changed", async () => {
    let finishResolution: ((item: ReviewItem) => void) | undefined;
    let finishReloadReviews: ((items: ReviewItem[]) => void) | undefined;
    const rejectedReviewItem: ReviewItem = {
      ...reviewItem,
      decision: { decision: "reject" },
      status: "rejected",
    };
    invokeMock.mockResolvedValueOnce([entry]).mockResolvedValueOnce([reviewItem]);
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (1)" }));
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<ReviewItem>((resolve) => {
          finishResolution = resolve;
        }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Reject candidate" }));

    // The reload reads the library before the decision detaches its asset.
    invokeMock.mockResolvedValueOnce([entry]).mockImplementationOnce(
      () =>
        new Promise<ReviewItem[]>((resolve) => {
          finishReloadReviews = resolve;
        }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    finishResolution?.(rejectedReviewItem);
    invokeMock.mockResolvedValueOnce([rejectedReviewItem]);
    expect(await screen.findByText("Rejected")).toBeInTheDocument();

    finishReloadReviews?.([reviewItem]);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled();
    });
    expect(screen.getByRole("button", { name: "Library (0)" })).toBeInTheDocument();
  });
});

describe("App acquisition", () => {
  const startedRun = {
    id: 1,
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
    queued_work: 0,
    awaiting_review_work: 0,
    completed_work: 0,
    below_quality_work: 0,
  };

  beforeEach(() => {
    invokeMock.mockReset();
    openVaultMock.mockReset();
    openVaultMock.mockResolvedValue(undefined);
  });

  it("creates a vault when asked instead of only opening an existing one", async () => {
    invokeMock.mockResolvedValue([]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Create vault" }));

    await waitFor(() =>
      expect(openVaultMock).toHaveBeenCalledWith({
        vault_root: ".game-media-vault",
        create: true,
      }),
    );
  });

  it("starts a run from the Acquire view and shows it in the Runs view", async () => {
    let runs: unknown[] = [];
    invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "start_acquisition_run") {
        runs = [startedRun];
        return Promise.resolve(startedRun);
      }
      if (command === "list_acquisition_runs") {
        return Promise.resolve(runs);
      }
      if (command === "list_library" || command === "list_review_items") {
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected command: ${command} ${JSON.stringify(args)}`));
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());

    fireEvent.click(screen.getByRole("button", { name: "Acquire" }));
    fireEvent.click(screen.getByLabelText("Libretro Thumbnails"));
    fireEvent.change(screen.getByLabelText("Platforms (one per line)"), {
      target: { value: "Nintendo - Game Boy" },
    });
    fireEvent.change(screen.getByLabelText("Games (one per line, empty for all)"), {
      target: { value: "Tetris (World) (Rev 1)" },
    });
    fireEvent.click(screen.getByLabelText("Box Front"));
    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));

    expect(await screen.findByRole("article", { name: "Run #1" })).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("start_acquisition_run", {
      request: startedRun.request,
    });
  });

  it("shows the shared validator message when the backend rejects a request", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "start_acquisition_run") {
        return Promise.reject({
          kind: "invalid_request",
          message: "acquisition request must include at least one source",
        });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());

    fireEvent.click(screen.getByRole("button", { name: "Acquire" }));
    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));

    expect(
      await screen.findByText("acquisition request must include at least one source"),
    ).toBeInTheDocument();
  });

  it("executes a run and refreshes the runs, library and reviews", async () => {
    let executed = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([
          executed ? { ...startedRun, status: "completed", completed_work: 1 } : startedRun,
        ]);
      }
      if (command === "execute_acquisition_run") {
        executed = true;
        return Promise.resolve({ ...startedRun, status: "completed", completed_work: 1 });
      }
      if (command === "list_library") {
        return Promise.resolve(executed ? [entry] : []);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));

    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));

    expect(await screen.findByText("Completed")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("execute_acquisition_run", {
      run_id: 1,
      matching_policy: { high_confidence_threshold: 80, medium_confidence_threshold: 50 },
    });
    fireEvent.click(screen.getByRole("button", { name: /Library/ }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
  });

  it("keeps pause available while a run executes", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([startedRun]);
      }
      if (command === "execute_acquisition_run") {
        return new Promise(() => {});
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));

    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));

    expect(await screen.findByRole("button", { name: "Executing…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Pause" })).toBeEnabled();
  });

  it("refreshes the progress an execution persisted before failing", async () => {
    let executed = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([executed ? { ...startedRun, completed_work: 1 } : startedRun]);
      }
      if (command === "execute_acquisition_run") {
        executed = true;
        return Promise.reject({ kind: "external", message: "download failed" });
      }
      if (command === "list_library") {
        return Promise.resolve(executed ? [entry] : []);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));

    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));

    expect(await screen.findByText("download failed")).toBeInTheDocument();
    expect(await screen.findByText("0 queued · 0 awaiting review · 1 completed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Library (1)" })).toBeInTheDocument();
  });

  it("keeps the execution failure when the refresh after it fails too", async () => {
    let executed = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([startedRun]);
      }
      if (command === "execute_acquisition_run") {
        executed = true;
        return Promise.reject({ kind: "external", message: "download failed" });
      }
      if (command === "list_library" && executed) {
        return Promise.reject({ kind: "external", message: "catalog busy" });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));

    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));

    expect(
      await screen.findByText("download failed (the vault could not be refreshed: catalog busy)"),
    ).toBeInTheDocument();
  });

  it("does not let an execution refresh overwrite a newer review decision", async () => {
    let finishExecutionRefresh: ((items: ReviewItem[]) => void) | undefined;
    let executed = false;
    const rejected: ReviewItem = { ...reviewItem, decision: { decision: "reject" }, status: "rejected" };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([startedRun]);
      }
      if (command === "execute_acquisition_run") {
        executed = true;
        return Promise.resolve(startedRun);
      }
      if (command === "list_review_items") {
        if (!executed) {
          return Promise.resolve([reviewItem]);
        }
        if (finishExecutionRefresh === undefined) {
          // The execution refresh read the item before the decision below.
          return new Promise<ReviewItem[]>((resolve) => {
            finishExecutionRefresh = resolve;
          });
        }
        return Promise.resolve([rejected]);
      }
      if (command === "resolve_review_item") {
        return Promise.resolve(rejected);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));
    await waitFor(() => expect(finishExecutionRefresh).toBeDefined());

    fireEvent.click(screen.getByRole("button", { name: "Review (1)" }));
    fireEvent.click(screen.getByRole("button", { name: "Reject candidate" }));
    expect(await screen.findByText("Rejected")).toBeInTheDocument();
    await act(async () => {
      finishExecutionRefresh?.([reviewItem]);
    });

    expect(screen.getByText("Rejected")).toBeInTheDocument();
  });

  it("keeps another vault's execution running when an older one finishes", async () => {
    const executions: Array<() => void> = [];
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([startedRun]);
      }
      if (command === "execute_acquisition_run") {
        return new Promise<void>((resolve) => executions.push(() => resolve()));
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));
    await waitFor(() => expect(executions).toHaveLength(1));

    fireEvent.change(screen.getByLabelText("Vault path"), { target: { value: "other-vault" } });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));
    await waitFor(() => expect(executions).toHaveLength(2));
    await act(async () => executions[0]());

    expect(screen.getByRole("button", { name: "Executing…" })).toBeDisabled();
  });

  it("keeps a run executing when its vault is loaded again", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([startedRun]);
      }
      if (command === "execute_acquisition_run") {
        return new Promise(() => {});
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));
    await screen.findByRole("button", { name: "Executing…" });

    fireEvent.change(screen.getByLabelText("Vault path"), { target: { value: "other-vault" } });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await screen.findByRole("button", { name: "Execute" });
    fireEvent.change(screen.getByLabelText("Vault path"), {
      target: { value: ".game-media-vault" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));

    // The backend still executes the run, so it must not be started a second time.
    expect(await screen.findByRole("button", { name: "Executing…" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Execute" })).not.toBeInTheDocument();
  });

  it("polls an executing run only once its vault is open again", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      invokeMock.mockImplementation((command: string) => {
        if (command === "list_acquisition_runs") {
          return Promise.resolve([startedRun]);
        }
        if (command === "get_acquisition_run") {
          return Promise.resolve(startedRun);
        }
        if (command === "execute_acquisition_run") {
          return new Promise(() => {});
        }
        return Promise.resolve([]);
      });
      render(<App />);
      fireEvent.click(screen.getByRole("button", { name: "Runs" }));
      fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
      fireEvent.click(await screen.findByRole("button", { name: "Execute" }));
      await screen.findByRole("button", { name: "Executing…" });
      fireEvent.change(screen.getByLabelText("Vault path"), { target: { value: "other-vault" } });
      fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
      await screen.findByRole("button", { name: "Execute" });

      let finishOpen: (() => void) | undefined;
      openVaultMock.mockImplementationOnce(
        () =>
          new Promise<void>((resolve) => {
            finishOpen = resolve;
          }),
      );
      fireEvent.change(screen.getByLabelText("Vault path"), {
        target: { value: ".game-media-vault" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
      invokeMock.mockClear();
      await act(async () => {
        vi.advanceTimersByTime(RUN_PROGRESS_REFRESH_MS);
      });

      // The backend still has the other vault open.
      expect(invokeMock).not.toHaveBeenCalledWith("get_acquisition_run", expect.anything());
      await act(async () => finishOpen?.());
      await act(async () => {
        vi.advanceTimersByTime(RUN_PROGRESS_REFRESH_MS);
      });
      expect(invokeMock).toHaveBeenCalledWith("get_acquisition_run", { run_id: 1 });
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not let an execution finishing in another vault discard this vault's runs", async () => {
    let finishExecution: (() => void) | undefined;
    let finishOtherListing: ((runs: unknown[]) => void) | undefined;
    let otherVaultOpened = false;
    openVaultMock.mockImplementation((args: { vault_root: string }) => {
      otherVaultOpened = args.vault_root === "other-vault";
      return Promise.resolve();
    });
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        if (otherVaultOpened && finishOtherListing === undefined) {
          return new Promise((resolve) => {
            finishOtherListing = resolve;
          });
        }
        return Promise.resolve([startedRun]);
      }
      if (command === "execute_acquisition_run") {
        return new Promise<void>((resolve) => {
          finishExecution = resolve;
        });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));
    await waitFor(() => expect(finishExecution).toBeDefined());
    fireEvent.change(screen.getByLabelText("Vault path"), { target: { value: "other-vault" } });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(finishOtherListing).toBeDefined());

    await act(async () => finishExecution?.());
    await act(async () => finishOtherListing?.([{ ...startedRun, id: 5 }]));

    expect(await screen.findByRole("article", { name: "Run #5" })).toBeInTheDocument();
  });

  it("does not let progress polls discard a full run list refresh", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      const secondRun = { ...startedRun, id: 2 };
      let finishListing: ((runs: unknown[]) => void) | undefined;
      let paused = false;
      invokeMock.mockImplementation((command: string) => {
        if (command === "list_acquisition_runs") {
          if (paused) {
            return new Promise((resolve) => {
              finishListing = resolve;
            });
          }
          return Promise.resolve([startedRun, secondRun]);
        }
        if (command === "pause_acquisition_run") {
          paused = true;
          return Promise.resolve({ ...secondRun, status: "paused" });
        }
        if (command === "get_acquisition_run") {
          return Promise.resolve({ ...startedRun, completed_work: 2 });
        }
        if (command === "execute_acquisition_run") {
          return new Promise(() => {});
        }
        return Promise.resolve([]);
      });
      render(<App />);
      fireEvent.click(screen.getByRole("button", { name: "Runs" }));
      fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
      await screen.findByRole("article", { name: "Run #2" });
      fireEvent.click(screen.getAllByRole("button", { name: "Execute" })[0]);
      fireEvent.click(screen.getAllByRole("button", { name: "Pause" })[1]);
      await act(async () => {});
      expect(finishListing).toBeDefined();

      // A poll of the executing run starts and settles while the full refresh is pending.
      await act(async () => {
        vi.advanceTimersByTime(RUN_PROGRESS_REFRESH_MS);
      });
      await act(async () =>
        finishListing?.([
          { ...startedRun, completed_work: 2 },
          { ...secondRun, status: "paused" },
        ]),
      );

      expect(screen.getByText("Paused")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows a run action that succeeded even when the run list cannot be refreshed", async () => {
    let paused = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return paused
          ? Promise.reject({ kind: "external", message: "catalog busy" })
          : Promise.resolve([startedRun]);
      }
      if (command === "pause_acquisition_run") {
        paused = true;
        return Promise.resolve({ ...startedRun, status: "paused" });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));

    fireEvent.click(await screen.findByRole("button", { name: "Pause" }));

    expect(await screen.findByText("Paused")).toBeInTheDocument();
    expect(
      screen.getByText("Run #1 is paused, but the run list could not be refreshed: catalog busy"),
    ).toBeInTheDocument();
  });

  it("keeps another vault's run action pending when an older one finishes", async () => {
    const pauses: Array<() => void> = [];
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([startedRun]);
      }
      if (command === "pause_acquisition_run") {
        return new Promise<void>((resolve) => pauses.push(() => resolve()));
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Pause" }));
    await waitFor(() => expect(pauses).toHaveLength(1));

    fireEvent.change(screen.getByLabelText("Vault path"), { target: { value: "other-vault" } });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Pause" }));
    await waitFor(() => expect(pauses).toHaveLength(2));
    await act(async () => pauses[0]());

    expect(screen.getByRole("button", { name: "Pause" })).toBeDisabled();
  });

  it("refreshes run counts while a run executes", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      let completed = 0;
      invokeMock.mockImplementation((command: string) => {
        if (command === "list_acquisition_runs") {
          return Promise.resolve([startedRun]);
        }
        if (command === "get_acquisition_run") {
          return Promise.resolve({ ...startedRun, completed_work: completed });
        }
        if (command === "execute_acquisition_run") {
          return new Promise(() => {});
        }
        return Promise.resolve([]);
      });
      render(<App />);
      fireEvent.click(screen.getByRole("button", { name: "Runs" }));
      fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
      fireEvent.click(await screen.findByRole("button", { name: "Execute" }));

      completed = 3;
      await act(async () => {
        vi.advanceTimersByTime(RUN_PROGRESS_REFRESH_MS);
      });

      expect(
        await screen.findByText("0 queued · 0 awaiting review · 3 completed"),
      ).toBeInTheDocument();
      expect(invokeMock).toHaveBeenCalledWith("get_acquisition_run", { run_id: 1 });
    } finally {
      vi.useRealTimers();
    }
  });

  it("refreshes runs after a review decision", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_review_items") {
        return Promise.resolve([reviewItem]);
      }
      if (command === "resolve_review_item") {
        return Promise.resolve({ ...reviewItem, decision: { decision: "reject" }, status: "rejected" });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Review (1)" }));
    invokeMock.mockClear();

    fireEvent.click(screen.getByRole("button", { name: "Reject candidate" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("list_acquisition_runs"));
  });

  it("lists the runs when Runs is opened while the vault loads", async () => {
    let finishOpen: (() => void) | undefined;
    openVaultMock.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finishOpen = resolve;
        }),
    );
    invokeMock.mockImplementation((command: string) =>
      Promise.resolve(command === "list_acquisition_runs" ? [startedRun] : []),
    );
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));

    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    await act(async () => finishOpen?.());

    expect(await screen.findByRole("article", { name: "Run #1" })).toBeInTheDocument();
  });

  it("keeps the newest run list when refreshes finish out of order", async () => {
    const listings: Array<(runs: unknown[]) => void> = [];
    const secondRun = { ...startedRun, id: 2 };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        if (listings.length === 0) {
          listings.push(() => {});
          return Promise.resolve([startedRun, secondRun]);
        }
        return new Promise((resolve) => listings.push(resolve));
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    await screen.findByRole("article", { name: "Run #2" });
    const [pauseFirst, pauseSecond] = screen.getAllByRole("button", { name: "Pause" });

    fireEvent.click(pauseFirst);
    await waitFor(() => expect(listings).toHaveLength(2));
    fireEvent.click(pauseSecond);
    await waitFor(() => expect(listings).toHaveLength(3));
    await act(async () =>
      listings[2]([
        { ...startedRun, status: "paused" },
        { ...secondRun, status: "paused" },
      ]),
    );
    await act(async () => listings[1]([{ ...startedRun, status: "paused" }, secondRun]));

    expect(screen.getAllByText("Paused")).toHaveLength(2);
  });

  it("keeps the newest library when acquisition refreshes finish out of order", async () => {
    const libraries: Array<(entries: unknown[]) => void> = [];
    const secondRun = { ...startedRun, id: 2 };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([startedRun, secondRun]);
      }
      if (command === "list_library" && libraries.length < 3) {
        if (libraries.length === 0) {
          libraries.push(() => {});
          return Promise.resolve([]);
        }
        return new Promise((resolve) => libraries.push(resolve));
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await screen.findByRole("article", { name: "Run #2" });
    const [executeFirst, executeSecond] = screen.getAllByRole("button", { name: "Execute" });

    fireEvent.click(executeFirst);
    await waitFor(() => expect(libraries).toHaveLength(2));
    fireEvent.click(executeSecond);
    await waitFor(() => expect(libraries).toHaveLength(3));
    await act(async () => libraries[2]([entry]));
    await act(async () => libraries[1]([]));

    expect(screen.getByRole("button", { name: "Library (1)" })).toBeInTheDocument();
  });

  it("shows a started run even when the run list cannot be refreshed", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "start_acquisition_run") {
        return Promise.resolve(startedRun);
      }
      if (command === "list_acquisition_runs") {
        return Promise.reject({ kind: "external", message: "catalog busy" });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Acquire" }));

    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));

    expect(await screen.findByRole("article", { name: "Run #1" })).toBeInTheDocument();
    expect(
      screen.getByText("Run #1 started, but the run list could not be refreshed: catalog busy"),
    ).toBeInTheDocument();
  });

  it("does not report a start failure in another vault", async () => {
    let failStart: ((reason: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "start_acquisition_run") {
        return new Promise((_resolve, reject) => {
          failStart = reject;
        });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Acquire" }));
    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));
    await waitFor(() => expect(failStart).toBeDefined());

    fireEvent.change(screen.getByLabelText("Vault path"), { target: { value: "other-vault" } });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    await act(async () => failStart?.({ kind: "external", message: "catalog busy" }));

    expect(screen.queryByText("catalog busy")).not.toBeInTheDocument();
  });

  it("does not report a run list failure in another vault", async () => {
    let failListing: ((reason: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs" && failListing === undefined) {
        return new Promise((_resolve, reject) => {
          failListing = reject;
        });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    await waitFor(() => expect(failListing).toBeDefined());

    fireEvent.change(screen.getByLabelText("Vault path"), { target: { value: "other-vault" } });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    await act(async () => failListing?.({ kind: "external", message: "catalog busy" }));

    expect(screen.queryByText("catalog busy")).not.toBeInTheDocument();
  });

  it("lists the runs of a vault loaded from the Runs view", async () => {
    invokeMock.mockImplementation((command: string) =>
      Promise.resolve(command === "list_acquisition_runs" ? [startedRun] : []),
    );
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));

    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));

    expect(await screen.findByRole("article", { name: "Run #1" })).toBeInTheDocument();
  });

  it("asks for a vault before starting an acquisition", async () => {
    invokeMock.mockResolvedValue([]);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Acquire" }));
    fireEvent.click(screen.getByRole("button", { name: "Start acquisition" }));

    expect(
      await screen.findByText("Load a vault before starting an acquisition."),
    ).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("start_acquisition_run", expect.anything());
  });
});

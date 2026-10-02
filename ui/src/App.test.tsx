import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LibraryEntry, ReviewItem } from "./types";

const { invokeMock, openVaultMock, libraryQueries } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  openVaultMock: vi.fn(),
  libraryQueries: [] as Record<string, unknown>[],
}));

/**
 * Tests script Library listings as `list_library` arrays; the App searches pages of them, so
 * a search answers with the scripted listing as a single page and records its query. A
 * scripted value that is already a page is returned as is.
 */
function searchLibrary(query: Record<string, unknown>) {
  libraryQueries.push(query);
  return Promise.resolve(invokeMock("list_library")).then((listing: unknown) =>
    Array.isArray(listing)
      ? { releases: listing, total: listing.length, next_after: null, as_of: 0 }
      : listing,
  );
}

// Opening the vault session is mocked separately so each test can script the data commands
// in the order the App issues them. Like the backend, it answers with the vault's identity,
// which tests take to be the path as typed unless they script another one.
vi.mock("@tauri-apps/api/core", () => ({
  // Mirrors how Tauri addresses custom protocols on Windows.
  convertFileSrc: (path: string, protocol: string) => "http://" + protocol + ".localhost/" + path,
  invoke: (command: string, args?: Record<string, unknown>) =>
    command === "open_vault"
      ? openVaultMock(args)
      : command === "search_library"
        ? searchLibrary(args?.query as Record<string, unknown>)
        : invokeMock(command, ...(args === undefined ? [] : [args])),
}));

import { App, RUN_PROGRESS_REFRESH_MS } from "./App";

/** Opens a vault whose identity is the path as typed. */
function sameIdentity(args: { vault_root: string }) {
  return Promise.resolve(args.vault_root);
}

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
  coverage: null,
  packaging_model: null,
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
      derived: [],
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

// The entry once its Box Front has a Library thumbnail.
const withLibraryThumbnail: LibraryEntry = {
  ...entry,
  assets: [
    {
      ...entry.assets[0],
      derived: [
        {
          recipe: { transform: "thumbnail", max_edge: 256 },
          object_hash: "thumb256",
          byte_len: 512,
          media_type: "image/png",
          width: 192,
          height: 256,
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
    openVaultMock.mockImplementation(sameIdentity);
  });

  it("searches the Library with the filters entered", async () => {
    invokeMock.mockImplementation(() => Promise.resolve([]));
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Load vault" })).toBeEnabled());
    libraryQueries.length = 0;

    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "mario" } });
    fireEvent.change(screen.getByLabelText("Platforms (one per line)"), {
      target: { value: "Nintendo - Game Boy\n\n Nintendo - Game Boy Color " },
    });
    fireEvent.change(screen.getByLabelText("Regions (one per line)"), {
      target: { value: "USA, Europe" },
    });
    fireEvent.change(screen.getByLabelText("Sources (one per line)"), {
      target: { value: "no-intro" },
    });
    const assetTypes = screen.getByLabelText("Asset Types") as HTMLSelectElement;
    for (const option of Array.from(assetTypes.options)) {
      option.selected = option.value === "box_front" || option.value === "screenshot";
    }
    fireEvent.change(assetTypes);
    fireEvent.click(screen.getByLabelText("Partial"));
    fireEvent.click(screen.getByRole("button", { name: "Search" }));

    await waitFor(() => expect(libraryQueries).toHaveLength(1));
    // Several values of one filter widen the search; the backend applies its default page size.
    expect(libraryQueries[0]).toEqual({
      text: "mario",
      platforms: ["Nintendo - Game Boy", "Nintendo - Game Boy Color"],
      regions: ["USA, Europe"],
      sources: ["no-intro"],
      asset_types: ["box_front", "screenshot"],
      statuses: ["partial"],
      after: null,
      as_of: null,
    });
  });

  it("does not page while a filter search is pending", async () => {
    let finishSearch: ((page: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_library") {
        if (libraryQueries.at(-1)?.text === "mario") {
          return new Promise((resolve) => {
            finishSearch = resolve;
          });
        }
        return Promise.resolve({ releases: [entry], total: 2, next_after: 2, as_of: 9 });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "mario" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(finishSearch).toBeDefined());

    expect(screen.getByRole("button", { name: "Load more" })).toBeDisabled();
  });

  it("ignores the failure of a search a newer one superseded", async () => {
    let failFirstSearch: ((reason: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_library") {
        if (libraryQueries.at(-1)?.text === "first") {
          return new Promise((_resolve, reject) => {
            failFirstSearch = reject;
          });
        }
        return Promise.resolve([entry]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "first" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(failFirstSearch).toBeDefined());
    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "second" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(libraryQueries.at(-1)).toMatchObject({ text: "second" }));
    await act(async () => failFirstSearch?.({ kind: "external", message: "catalog busy" }));

    expect(screen.queryByText("catalog busy")).not.toBeInTheDocument();
  });

  it("drops a page requested before a review decision refreshed the Library", async () => {
    const vagrantStory = { ...entry, release_edition_id: 9, game_title: "Vagrant Story" };
    let finishPage: ((page: unknown) => void) | undefined;
    const rejected = { ...reviewItem, decision: { decision: "reject" }, status: "rejected" };
    let decided = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_library") {
        if (libraryQueries.at(-1)?.after === 2) {
          return new Promise((resolve) => {
            finishPage = resolve;
          });
        }
        return Promise.resolve({ releases: [entry], total: 2, next_after: 2, as_of: 9 });
      }
      if (command === "list_review_items") {
        return Promise.resolve([decided ? rejected : reviewItem]);
      }
      if (command === "resolve_review_item") {
        decided = true;
        return Promise.resolve(rejected);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Load more" }));
    await waitFor(() => expect(finishPage).toBeDefined());

    fireEvent.click(screen.getByRole("button", { name: "Review (1)" }));
    fireEvent.click(screen.getByRole("button", { name: "Reject candidate" }));
    expect(await screen.findByText("Rejected")).toBeInTheDocument();
    await act(async () =>
      finishPage?.({ releases: [vagrantStory], total: 2, next_after: null, as_of: 9 }),
    );
    fireEvent.click(screen.getByRole("button", { name: /Library/ }));

    expect(screen.getByText("Metal Gear Solid")).toBeInTheDocument();
    expect(screen.queryByText("Vagrant Story")).not.toBeInTheDocument();
  });

  it("checks an acquisition plan through the desktop shell", async () => {
    invokeMock.mockImplementation((command: string) =>
      Promise.resolve(
        command === "plan_acquisition"
          ? {
              sources: [{ source_id: "libretro-thumbnails", asset_types: ["box_front"] }],
              excluded: [],
              coverage: [{ selector: "box_front", sources: ["libretro-thumbnails"] }],
            }
          : [],
      ),
    );
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Acquire" }));
    fireEvent.click(screen.getByLabelText("Box Front"));

    fireEvent.click(screen.getByRole("button", { name: "Check plan" }));

    expect(await screen.findByText("Box Front: Libretro Thumbnails")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith(
      "plan_acquisition",
      expect.objectContaining({ request: expect.objectContaining({ asset_types: ["box_front"] }) }),
    );
  });

  it("renders missing thumbnails and shows them in the Library", async () => {
    const thumbnail = {
      recipe: { transform: "thumbnail", max_edge: 256 },
      object_hash: "thumb256",
      byte_len: 512,
      media_type: "image/png",
      width: 192,
      height: 256,
    };
    let rendered = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "derive_thumbnails") {
        rendered = true;
        return Promise.resolve({
          derived: 1,
          skipped: 0,
          failed: [{ original_hash: "def456", reason: "cannot decode the original" }],
        });
      }
      if (command === "list_library") {
        return Promise.resolve([
          rendered ? { ...entry, assets: [{ ...entry.assets[0], derived: [thumbnail] }] } : entry,
        ]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Render thumbnails" }));

    expect(
      await screen.findByText("Rendered 1 thumbnail; 1 original could not be rendered."),
    ).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("derive_thumbnails", { max_edge: 256 });
    expect(screen.getByRole("img", { name: "Box Front of Metal Gear Solid" })).toHaveAttribute(
      "src",
      "http://gmv-object.localhost/thumb256",
    );
  });

  it("builds the 3D boxes the Library lacks and shows them", async () => {
    const model = {
      recipe: {
        transform: "packaging_model",
        template: "cardboard_box",
        back_hash: "back1",
        spine_hash: "spine1",
      },
      object_hash: "model1",
      byte_len: 4096,
      media_type: "model/gltf-binary",
      width: null,
      height: null,
    };
    let built = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "derive_packaging_models") {
        built = true;
        return Promise.resolve({
          generated: 1,
          up_to_date: 2,
          incomplete: [{ release_edition_id: 5, missing: ["spine"] }],
          without_template: 3,
          failed: [{ release_edition_id: 6, reason: "front scan: cannot decode the original" }],
        });
      }
      if (command === "list_library") {
        return Promise.resolve([built ? { ...entry, packaging_model: model } : entry]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Build 3D boxes" }));

    expect(
      await screen.findByText(
        "Built 1 3D box; 2 already built; 1 release misses scans; 3 releases have no 3D template; 1 release could not be built.",
      ),
    ).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("derive_packaging_models");
    expect(await screen.findByRole("figure", { name: "3D box of Metal Gear Solid" })).toBeInTheDocument();
  });

  it("reports the originals skipped as unsupported formats", async () => {
    invokeMock.mockImplementation((command: string) =>
      Promise.resolve(
        command === "derive_thumbnails"
          ? { derived: 0, skipped: 2, failed: [] }
          : command === "list_library"
            ? [entry]
            : [],
      ),
    );
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Render thumbnails" }));

    expect(
      await screen.findByText("Rendered 0 thumbnails; 2 originals skipped as unsupported formats."),
    ).toBeInTheDocument();
  });

  it("keeps a rendering attached to its vault while another vault renders its own", async () => {
    invokeMock.mockImplementation((command: string) =>
      command === "derive_thumbnails"
        ? new Promise(() => {})
        : Promise.resolve(command === "list_library" ? [entry] : []),
    );
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Render thumbnails" }));
    expect(await screen.findByRole("button", { name: "Rendering thumbnails…" })).toBeDisabled();

    // Reloading the same vault keeps its rendering; another vault can render its own.
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Rendering thumbnails…" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Vault path"), { target: { value: "other-vault" } });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));

    expect(await screen.findByRole("button", { name: "Render thumbnails" })).toBeEnabled();
  });

  it("releases the rendering once thumbnails are rendered, before the Library refresh", async () => {
    let rendered = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "derive_thumbnails") {
        rendered = true;
        return Promise.resolve({ derived: 1, skipped: 0, failed: [] });
      }
      if (command === "list_library") {
        // The refresh after the rendering never settles.
        return rendered ? new Promise(() => {}) : Promise.resolve([entry]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Render thumbnails" }));

    expect(await screen.findByText("Rendered 1 thumbnail.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Render thumbnails" })).toBeEnabled();
  });

  it("shows the thumbnails a failed rendering still produced", async () => {
    let rendered = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "derive_thumbnails") {
        rendered = true;
        return Promise.reject({ kind: "external", message: "disk full" });
      }
      if (command === "list_library") {
        return Promise.resolve([rendered ? withLibraryThumbnail : entry]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Render thumbnails" }));

    expect(await screen.findByText("disk full")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole("img", { name: "Box Front of Metal Gear Solid" })).toHaveAttribute(
        "src",
        "http://gmv-object.localhost/thumb256",
      ),
    );
  });

  it("shares a thumbnail rendering with every spelling of its vault's path", async () => {
    let finishRendering: ((summary: unknown) => void) | undefined;
    openVaultMock.mockResolvedValue("C:/vaults/main");
    invokeMock.mockImplementation((command: string) =>
      command === "derive_thumbnails"
        ? new Promise((resolve) => {
            finishRendering = resolve;
          })
        : Promise.resolve(command === "list_library" ? [entry] : []),
    );
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Render thumbnails" }));
    await waitFor(() => expect(finishRendering).toBeDefined());

    fireEvent.change(screen.getByLabelText("Vault path"), {
      target: { value: "C:/VAULTS/main" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(openVaultMock).toHaveBeenCalledTimes(2));

    // The same vault cannot start a second rendering, and reports the one it runs.
    expect(await screen.findByRole("button", { name: "Rendering thumbnails…" })).toBeDisabled();
    await act(async () => finishRendering?.({ derived: 1, skipped: 0, failed: [] }));
    expect(await screen.findByText("Rendered 1 thumbnail.")).toBeInTheDocument();
  });

  it("keeps the thumbnails a rendering showed while the same vault reloaded", async () => {
    let rendered = false;
    let reviewListings = 0;
    let finishRendering: ((summary: unknown) => void) | undefined;
    let finishReload: ((items: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "derive_thumbnails") {
        return new Promise((resolve) => {
          finishRendering = (summary) => {
            rendered = true;
            resolve(summary);
          };
        });
      }
      if (command === "list_library") {
        return Promise.resolve([rendered ? withLibraryThumbnail : entry]);
      }
      if (command === "list_review_items") {
        reviewListings += 1;
        // The reload read its Library page before the rendering ended, and settles last.
        return reviewListings === 2
          ? new Promise((resolve) => {
              finishReload = resolve;
            })
          : Promise.resolve([]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Render thumbnails" }));
    await waitFor(() => expect(finishRendering).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    await waitFor(() => expect(finishReload).toBeDefined());

    await act(async () => finishRendering?.({ derived: 1, skipped: 0, failed: [] }));
    await act(async () => finishReload?.([]));

    expect(screen.getByRole("img", { name: "Box Front of Metal Gear Solid" })).toHaveAttribute(
      "src",
      "http://gmv-object.localhost/thumb256",
    );
  });

  it("keeps a filter search submitted while thumbnails render", async () => {
    let finishRendering: ((summary: unknown) => void) | undefined;
    let metalSearches = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "derive_thumbnails") {
        return new Promise((resolve) => {
          finishRendering = resolve;
        });
      }
      if (command === "list_library") {
        if (libraryQueries.at(-1)?.text === "metal") {
          metalSearches += 1;
          // The first search never settles; the rendering supersedes it.
          return metalSearches === 1 ? new Promise(() => {}) : Promise.resolve([entry]);
        }
        return Promise.resolve([entry]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Render thumbnails" }));
    await waitFor(() => expect(finishRendering).toBeDefined());
    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "metal" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(metalSearches).toBe(1));

    await act(async () => finishRendering?.({ derived: 1, skipped: 0, failed: [] }));

    // The refresh searches again with the submitted filters, which then apply.
    await waitFor(() => expect(metalSearches).toBe(2));
    await waitFor(() => expect(screen.getByLabelText("Search titles")).toHaveValue("metal"));
    expect(libraryQueries.at(-1)).toMatchObject({ text: "metal" });
  });

  it.each([
    ["status", false, "Rendered 1 thumbnail."],
    ["failure", true, "disk full"],
  ])(
    "reports the %s of a rendering that ends while its own vault reopens",
    async (_outcome, fails, shown) => {
      let settleRendering: (() => void) | undefined;
      let finishReopen: (() => void) | undefined;
      invokeMock.mockImplementation((command: string) => {
        if (command === "derive_thumbnails") {
          return new Promise((resolve, reject) => {
            settleRendering = () =>
              fails
                ? reject({ kind: "external", message: "disk full" })
                : resolve({ derived: 1, skipped: 0, failed: [] });
          });
        }
        return Promise.resolve(command === "list_library" ? [entry] : []);
      });
      render(<App />);
      fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
      expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Render thumbnails" }));
      await waitFor(() => expect(settleRendering).toBeDefined());
      openVaultMock.mockImplementationOnce(
        (args: { vault_root: string }) =>
          new Promise<string>((resolve) => {
            finishReopen = () => resolve(args.vault_root);
          }),
      );
      fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
      await waitFor(() => expect(finishReopen).toBeDefined());

      await act(async () => settleRendering?.());
      await act(async () => finishReopen?.());

      expect(await screen.findByText(shown)).toBeInTheDocument();
    },
  );

  it.each([false, true])(
    "shows nothing of another vault's rendering that ends while this one is loaded (fails: %s)",
    async (fails) => {
      let settleRendering: (() => void) | undefined;
      invokeMock.mockImplementation((command: string) => {
        if (command === "derive_thumbnails") {
          return new Promise((resolve, reject) => {
            settleRendering = () =>
              fails
                ? reject({ kind: "external", message: "disk full" })
                : resolve({ derived: 1, skipped: 0, failed: [] });
          });
        }
        return Promise.resolve(command === "list_library" ? [entry] : []);
      });
      render(<App />);
      fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
      expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Render thumbnails" }));
      await waitFor(() => expect(settleRendering).toBeDefined());
      fireEvent.change(screen.getByLabelText("Vault path"), { target: { value: "other-vault" } });
      fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
      expect(await screen.findByRole("button", { name: "Render thumbnails" })).toBeEnabled();

      await act(async () => settleRendering?.());

      expect(screen.queryByText("Rendered 1 thumbnail.")).not.toBeInTheDocument();
      expect(screen.queryByText("disk full")).not.toBeInTheDocument();
    },
  );

  it("leaves reporting to a filter search that replaced the refresh after a rendering", async () => {
    let finishRendering: ((summary: unknown) => void) | undefined;
    let failRefresh: ((reason: unknown) => void) | undefined;
    let rendered = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "derive_thumbnails") {
        return new Promise((resolve) => {
          finishRendering = (summary) => {
            rendered = true;
            resolve(summary);
          };
        });
      }
      if (command === "list_library") {
        // The refresh after the rendering fails once a filter search replaced it.
        return rendered && failRefresh === undefined
          ? new Promise((_, reject) => {
              failRefresh = reject;
            })
          : Promise.resolve([entry]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Render thumbnails" }));
    await waitFor(() => expect(finishRendering).toBeDefined());
    await act(async () => finishRendering?.({ derived: 1, skipped: 0, failed: [] }));
    await waitFor(() => expect(failRefresh).toBeDefined());
    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "metal" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(screen.getByLabelText("Search titles")).toHaveValue("metal"));

    await act(async () => failRefresh?.({ kind: "external", message: "catalog busy" }));

    expect(screen.queryByText("catalog busy")).not.toBeInTheDocument();
  });

  it("imports a reference catalog into the opened vault and shows its releases", async () => {
    let imported = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "import_reference_catalog") {
        imported = true;
        return Promise.resolve({ imported_releases: 1, skipped_records: 2 });
      }
      return Promise.resolve(command === "list_library" && imported ? [entry] : []);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.change(await screen.findByLabelText("Catalog file"), {
      target: { value: "D:/dats/nes.dat" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Import catalog" }));

    expect(
      await screen.findByText("Imported 1 release; 2 malformed records skipped."),
    ).toBeInTheDocument();
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("import_reference_catalog", {
      input: { kind: "no_intro", file: "D:/dats/nes.dat", max_games: 5000, mame_version: null },
    });
  });

  it("shows the releases a failed import persisted before failing", async () => {
    let attempted = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "import_reference_catalog") {
        // Earlier batches committed before a later one failed.
        attempted = true;
        return Promise.reject({ kind: "external", message: "disk full" });
      }
      return Promise.resolve(command === "list_library" && attempted ? [entry] : []);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.change(await screen.findByLabelText("Catalog file"), {
      target: { value: "nes.dat" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Import catalog" }));

    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    expect(screen.getByText("disk full")).toBeInTheDocument();
  });

  it("reports a reference catalog it could not import and keeps the Library", async () => {
    invokeMock.mockImplementation((command: string) =>
      command === "import_reference_catalog"
        ? Promise.reject({ kind: "source_failure", message: "invalid No-Intro XML: broken" })
        : Promise.resolve(command === "list_library" ? [entry] : []),
    );
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Catalog file"), { target: { value: "broken.dat" } });

    fireEvent.click(screen.getByRole("button", { name: "Import catalog" }));

    expect(await screen.findByText("invalid No-Intro XML: broken")).toBeInTheDocument();
    expect(screen.getByText("Metal Gear Solid")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import catalog" })).toBeEnabled();
  });

  it("keeps a reference import attached to its vault while another vault loads", async () => {
    invokeMock.mockImplementation((command: string) =>
      command === "import_reference_catalog"
        ? new Promise(() => {})
        : Promise.resolve(command === "list_library" ? [entry] : []),
    );
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.change(await screen.findByLabelText("Catalog file"), {
      target: { value: "nes.dat" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Import catalog" }));
    expect(await screen.findByRole("button", { name: "Importing…" })).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Vault path"), { target: { value: "other-vault" } });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByRole("button", { name: "Import catalog" })).toBeEnabled();
    fireEvent.change(screen.getByLabelText("Vault path"), {
      target: { value: ".game-media-vault" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));

    // The backend still imports into this vault, so a second import must not start.
    expect(await screen.findByRole("button", { name: "Importing…" })).toBeDisabled();
  });

  it("keeps a filter search submitted while a reference catalog imports", async () => {
    let finishImport: ((summary: unknown) => void) | undefined;
    let metalSearches = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "import_reference_catalog") {
        return new Promise((resolve) => {
          finishImport = resolve;
        });
      }
      if (command === "list_library") {
        if (libraryQueries.at(-1)?.text === "metal") {
          metalSearches += 1;
          // The first search never settles; the import ending supersedes it.
          return metalSearches === 1 ? new Promise(() => {}) : Promise.resolve([entry]);
        }
        return Promise.resolve([entry]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Catalog file"), { target: { value: "nes.dat" } });
    fireEvent.click(screen.getByRole("button", { name: "Import catalog" }));
    await waitFor(() => expect(finishImport).toBeDefined());
    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "metal" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(metalSearches).toBe(1));

    await act(async () => finishImport?.({ imported_releases: 1, skipped_records: 0 }));

    // The Library is searched again with the submitted filters, which then apply.
    await waitFor(() => expect(metalSearches).toBe(2));
    await waitFor(() => expect(screen.getByLabelText("Search titles")).toHaveValue("metal"));
    expect(libraryQueries.at(-1)).toMatchObject({ text: "metal" });
  });

  it("offers no reference catalog import before a vault is loaded", () => {
    render(<App />);

    expect(
      screen.queryByRole("form", { name: "Reference catalog import" }),
    ).not.toBeInTheDocument();
  });

  it("offers no Library search before a vault is loaded", () => {
    render(<App />);

    expect(screen.queryByRole("form", { name: "Library filters" })).not.toBeInTheDocument();
  });

  it("keeps the previous results and filters when a search fails", async () => {
    let failNextSearch = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_library") {
        if (failNextSearch) {
          failNextSearch = false;
          return Promise.reject({ kind: "external", message: "catalog busy" });
        }
        return Promise.resolve({ releases: [entry], total: 2, next_after: 2, as_of: 9 });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();

    failNextSearch = true;
    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "mario" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    expect(await screen.findByText("catalog busy")).toBeInTheDocument();
    // The filter bar shows the filters of the shown results again.
    expect(screen.getByLabelText("Search titles")).toHaveValue("");

    // The next page still continues the search that produced the shown results.
    fireEvent.click(screen.getByRole("button", { name: "Load more" }));
    await waitFor(() => expect(libraryQueries.at(-1)).toMatchObject({ after: 2 }));
    expect(libraryQueries.at(-1)).toMatchObject({ text: null });
    // A search that succeeds clears the earlier failure.
    await waitFor(() => expect(screen.queryByText("catalog busy")).not.toBeInTheDocument());
  });

  it("shows the filters of a search that ended while the Library was hidden", async () => {
    let finishSearch: ((page: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_library") {
        if (libraryQueries.at(-1)?.text === "metal") {
          return new Promise((resolve) => {
            finishSearch = resolve;
          });
        }
        return Promise.resolve([entry]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "metal" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(finishSearch).toBeDefined());

    fireEvent.click(screen.getByRole("button", { name: "Review (0)" }));
    fireEvent.click(screen.getByRole("button", { name: /Library/ }));
    await act(async () => finishSearch?.([entry]));

    expect(screen.getByLabelText("Search titles")).toHaveValue("metal");
  });

  it("requests one next page at a time", async () => {
    let finishPage: ((page: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_library") {
        if (libraryQueries.at(-1)?.after === 2) {
          return new Promise((resolve) => {
            finishPage = resolve;
          });
        }
        return Promise.resolve({ releases: [entry], total: 2, next_after: 2, as_of: 9 });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    libraryQueries.length = 0;

    const loadMore = screen.getByRole("button", { name: "Load more" });
    fireEvent.click(loadMore);
    fireEvent.click(loadMore);
    await waitFor(() => expect(finishPage).toBeDefined());

    expect(libraryQueries).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Loading more…" })).toBeDisabled();
  });

  it("loads the next page of releases", async () => {
    const vagrantStory = { ...entry, release_edition_id: 9, game_title: "Vagrant Story" };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_library") {
        return Promise.resolve(
          libraryQueries.at(-1)?.after === 2
            ? { releases: [vagrantStory], total: 2, next_after: null, as_of: 9 }
            : { releases: [entry], total: 2, next_after: 2, as_of: 9 },
        );
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    expect(screen.getByText("2 releases · 0 reviews")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Load more" }));

    expect(await screen.findByText("Vagrant Story")).toBeInTheDocument();
    expect(screen.getByText("Metal Gear Solid")).toBeInTheDocument();
    // The next page keeps to the releases the first page searched.
    expect(libraryQueries.at(-1)).toMatchObject({ after: 2, as_of: 9 });
    expect(screen.queryByRole("button", { name: "Load more" })).not.toBeInTheDocument();
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
    outranked_work: 0,
    unavailable_work: 0,
  };

  beforeEach(() => {
    invokeMock.mockReset();
    openVaultMock.mockReset();
    openVaultMock.mockImplementation(sameIdentity);
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

  it("shares a run executing in a vault with every spelling of its path", async () => {
    // Both spellings name one directory, which the backend identifies once.
    openVaultMock.mockResolvedValue("C:/vaults/main");
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

    fireEvent.change(screen.getByLabelText("Vault path"), {
      target: { value: "./.game-media-vault" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));

    await waitFor(() => expect(openVaultMock).toHaveBeenCalledTimes(2));
    expect(await screen.findByRole("button", { name: "Executing…" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Execute" })).not.toBeInTheDocument();
    // The field keeps the spelling the user typed.
    expect(screen.getByLabelText("Vault path")).toHaveValue("./.game-media-vault");
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
        (args: { vault_root: string }) =>
          new Promise<string>((resolve) => {
            finishOpen = () => resolve(args.vault_root);
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
      return Promise.resolve(args.vault_root);
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
      (args: { vault_root: string }) =>
        new Promise<string>((resolve) => {
          finishOpen = () => resolve(args.vault_root);
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

describe("App Library requests", () => {
  const runToExecute = {
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
    queued_work: 1,
    awaiting_review_work: 0,
    completed_work: 0,
    below_quality_work: 0,
    outranked_work: 0,
    unavailable_work: 0,
  };

  beforeEach(() => {
    invokeMock.mockReset();
    openVaultMock.mockReset();
    openVaultMock.mockImplementation(sameIdentity);
    libraryQueries.length = 0;
  });

  it("ignores the failure of a page a newer search superseded", async () => {
    let failPage: ((reason: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_library") {
        if (libraryQueries.at(-1)?.after === 2) {
          return new Promise((_resolve, reject) => {
            failPage = reject;
          });
        }
        return Promise.resolve({ releases: [entry], total: 2, next_after: 2, as_of: 9 });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Load more" }));
    await waitFor(() => expect(failPage).toBeDefined());

    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "metal" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(libraryQueries.at(-1)).toMatchObject({ text: "metal" }));
    await act(async () => failPage?.({ kind: "external", message: "catalog busy" }));

    expect(screen.queryByText("catalog busy")).not.toBeInTheDocument();
  });

  it("keeps the Review Items an execution refresh read while a filter search ran", async () => {
    let executed = false;
    let finishRefreshPage: ((page: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([runToExecute]);
      }
      if (command === "execute_acquisition_run") {
        executed = true;
        return Promise.resolve(runToExecute);
      }
      if (command === "list_review_items") {
        return Promise.resolve(executed ? [reviewItem] : []);
      }
      if (command === "list_library") {
        if (executed && libraryQueries.at(-1)?.text === null && !finishRefreshPage) {
          // The Library page of the refresh after the execution arrives late.
          return new Promise((resolve) => {
            finishRefreshPage = resolve;
          });
        }
        return Promise.resolve([entry]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));
    await waitFor(() => expect(finishRefreshPage).toBeDefined());

    fireEvent.click(screen.getByRole("button", { name: /Library/ }));
    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "metal" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(libraryQueries.at(-1)).toMatchObject({ text: "metal" }));
    await act(async () => finishRefreshPage?.([]));

    // The newer search keeps its page; the refresh still brings the run's Review Item.
    expect(screen.getByText("Metal Gear Solid")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Review (1)" })).toBeInTheDocument();
  });

  it("pages the results of a search that superseded a pending page", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_library") {
        const query = libraryQueries.at(-1);
        if (query?.after === 2 && query?.text === null) {
          // The page of the previous results never settles.
          return new Promise(() => {});
        }
        return Promise.resolve({ releases: [entry], total: 2, next_after: 2, as_of: 9 });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Metal Gear Solid")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Load more" }));
    expect(await screen.findByRole("button", { name: "Loading more…" })).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "metal" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    fireEvent.click(await screen.findByRole("button", { name: "Load more" }));

    await waitFor(() => expect(libraryQueries.at(-1)).toMatchObject({ text: "metal", after: 2 }));
  });

  it("drops a page that extends results a pending refresh then replaced", async () => {
    const vagrantStory = { ...entry, release_edition_id: 9, game_title: "Vagrant Story" };
    let executed = false;
    let finishRefreshPage: ((page: unknown) => void) | undefined;
    let finishPage: ((page: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([runToExecute]);
      }
      if (command === "execute_acquisition_run") {
        executed = true;
        return Promise.resolve(runToExecute);
      }
      if (command === "list_library") {
        const query = libraryQueries.at(-1);
        if (query?.after === 2) {
          return new Promise((resolve) => {
            finishPage = resolve;
          });
        }
        if (executed && !finishRefreshPage) {
          return new Promise((resolve) => {
            finishRefreshPage = resolve;
          });
        }
        return Promise.resolve({ releases: [entry], total: 2, next_after: 2, as_of: 9 });
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));
    await waitFor(() => expect(finishRefreshPage).toBeDefined());

    // The page continues the results shown before the execution refresh.
    fireEvent.click(screen.getByRole("button", { name: /Library/ }));
    fireEvent.click(screen.getByRole("button", { name: "Load more" }));
    await waitFor(() => expect(finishPage).toBeDefined());
    await act(async () =>
      finishRefreshPage?.({ releases: [entry], total: 1, next_after: null, as_of: 10 }),
    );
    await act(async () =>
      finishPage?.({ releases: [vagrantStory], total: 2, next_after: null, as_of: 9 }),
    );

    expect(screen.getByText("Metal Gear Solid")).toBeInTheDocument();
    expect(screen.queryByText("Vagrant Story")).not.toBeInTheDocument();
  });

  it("shows the applied filters again when a refresh supersedes a pending search", async () => {
    let finishExecution: ((run: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_acquisition_runs") {
        return Promise.resolve([runToExecute]);
      }
      if (command === "execute_acquisition_run") {
        return new Promise((resolve) => {
          finishExecution = resolve;
        });
      }
      if (command === "list_library") {
        if (libraryQueries.at(-1)?.text === "mario") {
          // The search never settles; the refresh after the execution supersedes it.
          return new Promise(() => {});
        }
        return Promise.resolve([entry]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Execute" }));
    await waitFor(() => expect(finishExecution).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: /Library/ }));
    fireEvent.change(screen.getByLabelText("Search titles"), { target: { value: "mario" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(libraryQueries.at(-1)).toMatchObject({ text: "mario" }));

    await act(async () => finishExecution?.(runToExecute));

    // The refresh searched with the applied filters, which the filter bar shows again.
    await waitFor(() => expect(libraryQueries.at(-1)).toMatchObject({ text: null }));
    await waitFor(() => expect(screen.getByLabelText("Search titles")).toHaveValue(""));
  });

  it("shows the registered Sources without a vault", async () => {
    invokeMock.mockImplementation((command: string) =>
      Promise.resolve(
        command === "list_sources"
          ? [
              {
                source_id: "launchbox-games-db",
                asset_types: ["box_front", "logo"],
                direct_media_download: true,
              },
            ]
          : [],
      ),
    );
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Sources" }));

    expect(
      await screen.findByRole("region", { name: "LaunchBox Games Database" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Box Front, Logo")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("list_sources");
  });

  it("reports Sources it could not read and reads them again when shown again", async () => {
    let failing = true;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_sources") {
        return failing
          ? Promise.reject({ kind: "external", message: "registry unavailable" })
          : Promise.resolve([
              { source_id: "libretro-thumbnails", asset_types: [], direct_media_download: true },
            ]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Sources" }));
    expect(await screen.findByText("registry unavailable")).toBeInTheDocument();

    failing = false;
    fireEvent.click(screen.getByRole("button", { name: /Library/ }));
    fireEvent.click(screen.getByRole("button", { name: "Sources" }));

    expect(await screen.findByRole("region", { name: "Libretro Thumbnails" })).toBeInTheDocument();
    // The failure no longer shows once the Sources are read.
    expect(screen.queryByText("registry unavailable")).not.toBeInTheDocument();
  });

  it("keeps the Sources a newer read shows when an older read fails late", async () => {
    let failFirstRead: ((reason: unknown) => void) | undefined;
    let reads = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_sources") {
        reads += 1;
        return reads === 1
          ? new Promise((_, reject) => {
              failFirstRead = reject;
            })
          : Promise.resolve([
              { source_id: "libretro-thumbnails", asset_types: [], direct_media_download: true },
            ]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Sources" }));
    fireEvent.click(screen.getByRole("button", { name: "Sources" }));
    expect(await screen.findByRole("region", { name: "Libretro Thumbnails" })).toBeInTheDocument();

    await act(async () => failFirstRead?.({ kind: "external", message: "registry unavailable" }));

    expect(screen.getByRole("region", { name: "Libretro Thumbnails" })).toBeInTheDocument();
    expect(screen.queryByText("registry unavailable")).not.toBeInTheDocument();
  });

  it("shows the failures the loaded vault recorded for each Source", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_sources") {
        return Promise.resolve([
          { source_id: "libretro-thumbnails", asset_types: ["box_front"], direct_media_download: true },
        ]);
      }
      if (command === "list_source_failures") {
        return Promise.resolve([
          {
            source_id: "libretro-thumbnails",
            failures: 1,
            latest: [
              {
                sequence: 1,
                source_id: "libretro-thumbnails",
                run_id: 7,
                stage: "discovery",
                message: "timed out",
                recorded_at: 1_790_000_000,
              },
            ],
          },
        ]);
      }
      return Promise.resolve([]);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Load vault" }));
    expect(await screen.findByText("Library is empty")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Sources" }));

    expect(await screen.findByText("1 failure recorded")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("list_source_failures", { latest: 3 });
  });

  it("reads no failures before a vault is loaded", async () => {
    invokeMock.mockImplementation((command: string) =>
      Promise.resolve(
        command === "list_sources"
          ? [{ source_id: "libretro-thumbnails", asset_types: [], direct_media_download: true }]
          : [],
      ),
    );
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Sources" }));

    expect(await screen.findByRole("region", { name: "Libretro Thumbnails" })).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("list_source_failures", expect.anything());
    expect(screen.queryByText("No failure recorded")).not.toBeInTheDocument();
  });
});

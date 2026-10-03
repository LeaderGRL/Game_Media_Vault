import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ReferenceReviewView } from "./ReferenceReviewView";
import type { ReferenceReviewItem } from "./types";

const item: ReferenceReviewItem = {
  id: 4,
  source_id: "redump",
  source_record: "nintendo - game boy|Game C (World)",
  release_edition_id: 30,
  evidence: "sha1",
  candidates: [10, 20],
  editions: [
    {
      release_edition_id: 30,
      game_title: "Game C",
      platform: "Nintendo - Game Boy",
      region: "World",
      edition_name: "Standard",
      records: [
        { source_id: "redump", source_record: "nintendo - game boy|Game C (World)", title: "Game C" },
      ],
    },
    {
      release_edition_id: 10,
      game_title: "Game A",
      platform: "Nintendo - Game Boy",
      region: "World",
      edition_name: "Standard",
      records: [
        { source_id: "no-intro", source_record: "nintendo - game boy|Game A (World)", title: "Game A" },
      ],
    },
    {
      release_edition_id: 20,
      game_title: "Game B",
      platform: "Nintendo - Game Boy",
      region: "World",
      edition_name: "Standard",
      records: [
        { source_id: "no-intro", source_record: "nintendo - game boy|Game B (World)", title: "Game B" },
      ],
    },
  ],
};

describe("ReferenceReviewView", () => {
  it("shows the record, its evidence and each edition it may describe", () => {
    render(
      <ReferenceReviewView items={[item]} decidingIds={new Set()} onLink={vi.fn()} onKeepApart={vi.fn()} />,
    );

    const card = screen.getByRole("article", { name: "Reference review #4" });
    expect(within(card).getByRole("heading", { name: "Game C" })).toBeInTheDocument();
    expect(within(card).getByText("redump · nintendo - game boy|Game C (World)")).toBeInTheDocument();
    expect(within(card).getByText("Same dumps")).toBeInTheDocument();
    const candidates = within(card).getByRole("list", { name: "Candidate editions" });
    expect(within(candidates).getByRole("heading", { name: "Game A" })).toBeInTheDocument();
    expect(within(candidates).getByRole("heading", { name: "Game B" })).toBeInTheDocument();
    expect(within(candidates).getByText("no-intro · nintendo - game boy|Game A (World)")).toBeInTheDocument();
  });

  it("links the record to a candidate or keeps it apart", () => {
    const onLink = vi.fn();
    const onKeepApart = vi.fn();
    render(
      <ReferenceReviewView items={[item]} decidingIds={new Set()} onLink={onLink} onKeepApart={onKeepApart} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Same release as Game B" }));
    expect(onLink).toHaveBeenCalledWith(4, 20);

    fireEvent.click(screen.getByRole("button", { name: "Distinct release" }));
    expect(onKeepApart).toHaveBeenCalledWith(4);
  });

  it("disables the decisions of an item being decided", () => {
    render(
      <ReferenceReviewView items={[item]} decidingIds={new Set([4])} onLink={vi.fn()} onKeepApart={vi.fn()} />,
    );

    expect(screen.getByRole("button", { name: "Same release as Game A" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Distinct release" })).toBeDisabled();
  });

  it("names a candidate the catalog no longer describes by its id", () => {
    const missing: ReferenceReviewItem = { ...item, editions: item.editions.slice(0, 2) };
    render(
      <ReferenceReviewView items={[missing]} decidingIds={new Set()} onLink={vi.fn()} onKeepApart={vi.fn()} />,
    );

    expect(screen.getByRole("heading", { name: "Release #20" })).toBeInTheDocument();
  });
});

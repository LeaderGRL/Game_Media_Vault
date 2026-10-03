import { ANY_ASSET_TYPE, type AcquisitionRun, type GameSelection } from "./acquisition";
import { consoleName, languageName } from "./catalog";

/** How far a run went, as the Activity view shows it. */
export interface RunProgress {
  /** Share of the planned Sources' search done, from 0 to 1. */
  searched: number;
  /** Whether some planned Source still has games to look up. */
  searching: boolean;
  /** Planned Sources whose search is complete. */
  sourcesDone: number;
  sources: number;
  /** Media found so far, settled or not. */
  found: number;
  /** Media found and settled: kept, not found, skipped or matched. */
  settled: number;
  /** Media kept in the vault. */
  acquired: number;
  /** Media a Source listed but no longer serves. */
  notFound: number;
  /** Media awaiting a decision in the Review view. */
  toReview: number;
  /** Media below the quality asked for, or outranked by better ones kept. */
  skipped: number;
  /** Share of the media found that is settled, from 0 to 1. */
  downloaded: number;
}

/** The games a selection names, each once regardless of case, or none for every game. */
function gameCount(games: GameSelection): number {
  switch (games.mode) {
    case "all":
      return 0;
    case "explicit":
      return new Set(games.values.map((game) => game.trim().toLowerCase())).size;
    case "platform_bound":
    case "query_result":
      return new Set(
        games.values.map(
          (selector) => `${selector.game.trim().toLowerCase()}\u0000${selector.platform}`,
        ),
      ).size;
  }
}

export function runProgress(run: AcquisitionRun): RunProgress {
  const games = gameCount(run.request.games);
  const discoveries = run.discoveries ?? [];
  const finished = run.status === "completed";
  const shares = discoveries.map((discovery) => {
    if (discovery.complete || finished) {
      return 1;
    }
    return games > 0 ? Math.min(discovery.discovered_games / games, 1) : 0;
  });
  const searched =
    shares.length === 0
      ? finished
        ? 1
        : 0
      : shares.reduce((sum, share) => sum + share, 0) / shares.length;
  const found = run.queued_work + run.awaiting_review_work + run.completed_work;
  const skipped = run.below_quality_work + run.outranked_work;
  return {
    searched,
    searching: searched < 1 && run.status !== "cancelled" && !finished,
    sourcesDone: shares.filter((share) => share === 1).length,
    sources: shares.length,
    found,
    settled: run.completed_work,
    acquired: Math.max(run.completed_work - run.unavailable_work - skipped, 0),
    notFound: run.unavailable_work,
    toReview: run.awaiting_review_work,
    skipped,
    downloaded: found === 0 ? 0 : run.completed_work / found,
  };
}

/** A run named after the consoles it acquires for. */
export function runTitle(run: AcquisitionRun): string {
  return run.request.platforms.map(consoleName).join(" + ") || `Run #${run.id}`;
}

/** What a run keeps to, in a few words. */
export function describeRunRequest(run: AcquisitionRun): string {
  const request = run.request;
  const games = gameCount(request.games);
  const parts = [games === 0 ? "Every game" : `${games} ${games === 1 ? "game" : "games"}`];
  // Worldwide media serve every region, so a request kept to regions adds them by itself.
  const regions = request.regions.filter((region) => region !== "World");
  if (regions.length > 0) {
    parts.push(regions.join(", "));
  }
  if (request.languages.length > 0) {
    parts.push(request.languages.map(languageName).join(", "));
  }
  if (!request.asset_types.includes(ANY_ASSET_TYPE)) {
    const count = request.asset_types.length;
    parts.push(`${count} ${count === 1 ? "media type" : "media types"}`);
  }
  const retention = request.retention;
  if (retention === "keep_best_per_type") {
    parts.push("Best of each type");
  } else if (typeof retention === "object") {
    parts.push(`Best ${retention.keep_best.per_type} of each type`);
  }
  return parts.join(" · ");
}

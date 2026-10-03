import { type ReactNode, useMemo, useState } from "react";

import {
  ANY_ASSET_TYPE,
  ASSET_TYPE_FAMILIES,
  type AcquisitionLimits,
  type AcquisitionRequestDraft,
  type RetentionPolicy,
  type SourceDescription,
  sourceLabel,
} from "./acquisition";
import {
  COMMON_LANGUAGES,
  COMMON_REGIONS,
  CONSOLES,
  LANGUAGES,
  OTHER_LANGUAGES,
  OTHER_REGIONS,
  consoleName,
  consolesByMaker,
} from "./catalog";
import { ChipGroup, Segmented } from "./controls";
import { Icon } from "./icons";

type GameScope = "all" | "count" | "percent";

type Keep = "all" | "1" | "3" | "5" | "10";

const KEEP_OPTIONS: { value: Keep; label: string }[] = [
  { value: "all", label: "Everything" },
  { value: "1", label: "Best 1" },
  { value: "3", label: "Best 3" },
  { value: "5", label: "Best 5" },
  { value: "10", label: "Best 10" },
];

interface DownloadViewProps {
  /** The registered Sources, which tell the media types and Sources on offer, once read. */
  sources: SourceDescription[] | null;
  /** Starts one request per console chosen. */
  onStart: (requests: AcquisitionRequestDraft[]) => void;
  /** Requests the Download view does not cover, such as some games only. */
  advanced?: ReactNode;
}

/** Whether a Source takes part on this machine: enabled, with the credentials it needs. */
function usable(source: SourceDescription) {
  return source.enabled && (source.credential === "not_needed" || source.credential === "stored");
}

/** Why a Source takes no part on this machine, in a few words. */
function unusableReason(source: SourceDescription) {
  if (!source.enabled) {
    return "Disabled";
  }
  return "Needs a key";
}

/**
 * Downloads media for whole consoles: which consoles, how many of their games, which regions,
 * languages and media, and how many of each to keep, picked rather than typed.
 */
export function DownloadView({ sources, onStart, advanced }: DownloadViewProps) {
  const [query, setQuery] = useState("");
  const [consoles, setConsoles] = useState<string[]>([]);
  const [scope, setScope] = useState<GameScope>("all");
  const [gameCount, setGameCount] = useState(100);
  const [gamePercent, setGamePercent] = useState(50);
  const [regions, setRegions] = useState<string[]>([]);
  const [languages, setLanguages] = useState<string[]>([]);
  const [everyMedia, setEveryMedia] = useState(true);
  const [mediaTypes, setMediaTypes] = useState<string[]>([]);
  const [keep, setKeep] = useState<Keep>("all");
  const [everySource, setEverySource] = useState(true);
  const [chosenSources, setChosenSources] = useState<string[]>([]);

  const groups = useMemo(() => consolesByMaker(CONSOLES, query), [query]);
  // The Sources taking part: every usable one, or those ticked.
  const taking = useMemo(
    () =>
      (sources ?? []).filter(
        (source) => usable(source) && (everySource || chosenSources.includes(source.source_id)),
      ),
    [sources, everySource, chosenSources],
  );
  const offered = useMemo(() => mediaOffer(sources, taking), [sources, taking]);
  const available = new Set(
    offered.flatMap((family) =>
      family.types.filter((type) => type.available).map((type) => type.value),
    ),
  );

  function toggle(values: string[], value: string) {
    return values.includes(value) ? values.filter((item) => item !== value) : [...values, value];
  }

  function selectMaker(platforms: string[]) {
    const allSelected = platforms.every((platform) => consoles.includes(platform));
    setConsoles((current) =>
      allSelected
        ? current.filter((platform) => !platforms.includes(platform))
        : [...current, ...platforms.filter((platform) => !current.includes(platform))],
    );
  }

  const limits: AcquisitionLimits =
    scope === "count"
      ? { max_games: Math.max(1, Math.round(gameCount)) }
      : scope === "percent"
        ? { games_percent: Math.min(100, Math.max(1, Math.round(gamePercent))) }
        : {};
  const retention: RetentionPolicy =
    keep === "all" ? "keep_everything" : { keep_best: { per_type: Number(keep) } };
  // Types no Source taking part acquires stay out of the request, which planning would refuse.
  const assetTypes = everyMedia
    ? [ANY_ASSET_TYPE]
    : mediaTypes.filter((type) => available.has(type));
  const ready =
    consoles.length > 0 && assetTypes.length > 0 && (everySource || chosenSources.length > 0);

  function start() {
    onStart(
      consoles.map((platform) => ({
        sources: everySource ? { mode: "auto" } : { mode: "explicit", values: chosenSources },
        platforms: [platform],
        games: { mode: "all" },
        regions,
        languages,
        asset_types: assetTypes,
        quality: null,
        retention,
        limits,
      })),
    );
  }

  return (
    <div className="download-layout">
      <section className="card step" aria-labelledby="step-consoles">
        <div className="step-heading">
          <span className="step-number">1</span>
          <h2 id="step-consoles">Consoles</h2>
          <span className="hint">
            {consoles.length === 0
              ? "Tick one or more"
              : `${consoles.length} selected`}
          </span>
        </div>
        {consoles.length > 0 ? (
          <div className="selected-consoles" aria-label="Selected consoles">
            {consoles.map((platform) => (
              <span className="selected-pill" key={platform}>
                {consoleName(platform)}
                <button
                  type="button"
                  aria-label={`Remove ${consoleName(platform)}`}
                  onClick={() => setConsoles((current) => toggle(current, platform))}
                >
                  <Icon name="close" size={14} />
                </button>
              </span>
            ))}
            <button type="button" className="ghost" onClick={() => setConsoles([])}>
              Clear
            </button>
          </div>
        ) : null}
        <div className="search-field">
          <Icon name="search" size={18} />
          <input
            aria-label="Find a console"
            placeholder="Find a console, such as Super Nintendo or PlayStation"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </div>
        <div className="console-groups">
          {groups.length === 0 ? <p className="hint">No console matches.</p> : null}
          {groups.map((group) => {
            const platforms = group.consoles.map((console) => console.platform);
            return (
              <div key={group.maker}>
                <div className="console-group-heading">
                  {group.maker}
                  <button
                    type="button"
                    className="ghost"
                    aria-label={`Select every ${group.maker} console`}
                    onClick={() => selectMaker(platforms)}
                  >
                    {platforms.every((platform) => consoles.includes(platform))
                      ? "Unselect all"
                      : "Select all"}
                  </button>
                </div>
                <div className="console-grid">
                  {group.consoles.map((console) => {
                    const selected = consoles.includes(console.platform);
                    return (
                      <label
                        key={console.platform}
                        className={selected ? "console-option selected" : "console-option"}
                      >
                        <input
                          type="checkbox"
                          checked={selected}
                          onChange={() =>
                            setConsoles((current) => toggle(current, console.platform))
                          }
                        />
                        {console.name}
                      </label>
                    );
                  })}
                </div>
              </div>
            );
          })}
        </div>
      </section>

      <section className="card step" aria-labelledby="step-games">
        <div className="step-heading">
          <span className="step-number">2</span>
          <h2 id="step-games">Games</h2>
        </div>
        <div className="option-row">
          <Segmented
            label="Games"
            value={scope}
            onChange={setScope}
            options={[
              { value: "all", label: "All games" },
              { value: "count", label: "A number per console" },
              { value: "percent", label: "A share of each console" },
            ]}
          />
          {scope === "count" ? (
            <label className="stepper">
              <input
                type="number"
                min={1}
                aria-label="Games per console"
                value={gameCount}
                onChange={(event) => setGameCount(Number(event.target.value))}
              />
              games per console, by name
            </label>
          ) : null}
          {scope === "percent" ? (
            <label className="stepper">
              <input
                type="range"
                min={5}
                max={100}
                step={5}
                aria-hidden="true"
                tabIndex={-1}
                value={gamePercent}
                onChange={(event) => setGamePercent(Number(event.target.value))}
              />
              <input
                type="number"
                min={1}
                max={100}
                aria-label="Share of games (%)"
                value={gamePercent}
                onChange={(event) => setGamePercent(Number(event.target.value))}
              />
              % of each console's games, by name
            </label>
          ) : null}
        </div>
      </section>

      <section className="card step" aria-labelledby="step-regions">
        <div className="step-heading">
          <span className="step-number">3</span>
          <h2 id="step-regions">Regions and languages</h2>
          <span className="hint">Worldwide releases count with any region; World alone keeps to them</span>
        </div>
        <ChipGroup
          label="Regions"
          anyLabel="Any region"
          options={COMMON_REGIONS}
          more={OTHER_REGIONS}
          moreLabel="More regions"
          selected={regions}
          onChange={setRegions}
        />
        <ChipGroup
          label="Languages"
          anyLabel="Any language"
          options={COMMON_LANGUAGES}
          more={OTHER_LANGUAGES}
          moreLabel="More languages"
          selected={languages}
          onChange={setLanguages}
        />
      </section>

      <section className="card step" aria-labelledby="step-media">
        <div className="step-heading">
          <span className="step-number">4</span>
          <h2 id="step-media">Media</h2>
        </div>
        <div className="option-row">
          <label className="choice">
            <input
              type="checkbox"
              checked={everyMedia}
              onChange={(event) => setEveryMedia(event.target.checked)}
            />
            Every kind of media
          </label>
          <span className="hint">covers, spines, discs, manuals, maps, screenshots, videos…</span>
        </div>
        {everyMedia ? null : (
          <div className="media-families">
            {offered.map((family) => (
              <div className="media-family" key={family.value}>
                <h3>{family.label}</h3>
                {family.types.map((type) => (
                  <div className="choice-row" key={type.value}>
                    <label className={type.available ? "choice" : "choice unavailable"}>
                      <input
                        type="checkbox"
                        disabled={!type.available}
                        checked={type.available && mediaTypes.includes(type.value)}
                        onChange={() => setMediaTypes((current) => toggle(current, type.value))}
                      />
                      {type.label}
                    </label>
                    {type.note ? <span className="choice-note">{type.note}</span> : null}
                  </div>
                ))}
              </div>
            ))}
          </div>
        )}
        <div className="option-row">
          <span>Keep of each type, per game</span>
          <Segmented label="Keep of each type" value={keep} onChange={setKeep} options={KEEP_OPTIONS} />
        </div>
      </section>

      <section className="card step" aria-labelledby="step-sources">
        <div className="step-heading">
          <span className="step-number">5</span>
          <h2 id="step-sources">Sources</h2>
        </div>
        <div className="option-row">
          <label className="choice">
            <input
              type="checkbox"
              checked={everySource}
              onChange={(event) => setEverySource(event.target.checked)}
            />
            Use every available Source
          </label>
          <span className="hint">Sources needing a key take part once it is stored</span>
        </div>
        {everySource ? null : (
          <div className="chip-group">
            {(sources ?? []).map((source) => (
              <div
                key={source.source_id}
                className={usable(source) ? "console-option" : "console-option unavailable"}
              >
                <label className="choice">
                  <input
                    type="checkbox"
                    disabled={!usable(source)}
                    checked={chosenSources.includes(source.source_id)}
                    onChange={() =>
                      setChosenSources((current) => toggle(current, source.source_id))
                    }
                  />
                  {sourceLabel(source.source_id)}
                </label>
                {usable(source) ? null : (
                  <span className="choice-note">{unusableReason(source)}</span>
                )}
              </div>
            ))}
          </div>
        )}
      </section>

      <div className="summary-bar">
        <p className="summary-text" aria-live="polite">
          {consoles.length === 0 ? (
            "Choose at least one console to start."
          ) : (
            <>
              <strong>
                {consoles.length === 1 ? consoleName(consoles[0]) : `${consoles.length} consoles`}
              </strong>
              {" · "}
              {summarize(scope, gameCount, gamePercent, regions, languages, everyMedia, keep)}
            </>
          )}
        </p>
        <button
          type="button"
          className="primary large"
          disabled={!ready}
          onClick={start}
        >
          <Icon name="download" />
          Start download
        </button>
      </div>

      {advanced ? (
        <details className="card advanced-request">
          <summary>Advanced request: particular games, Sources or quality</summary>
          {advanced}
        </details>
      ) : null}
    </div>
  );
}

function summarize(
  scope: GameScope,
  gameCount: number,
  gamePercent: number,
  regions: string[],
  languages: string[],
  everyMedia: boolean,
  keep: Keep,
) {
  const parts = [
    scope === "all"
      ? "every game"
      : scope === "count"
        ? `${gameCount} games each`
        : `${gamePercent}% of the games`,
    regions.length === 0 ? "any region" : regions.join(", "),
    languages.length === 0
      ? "any language"
      : languages
          .map((code) => LANGUAGES.find((language) => language.value === code)?.label ?? code)
          .join(", "),
    everyMedia ? "every kind of media" : "chosen media",
    keep === "all" ? "keep everything" : `best ${keep} of each type`,
  ];
  return parts.join(" · ");
}

/** A media type on offer, and whether a Source taking part acquires it. */
interface OfferedType {
  value: string;
  label: string;
  available: boolean;
  /** Why no Source taking part acquires it, when none does. */
  note: string | null;
}

/**
 * The media types some registered Source acquires, by family, each marked available when a
 * Source taking part on this machine acquires it; every type before the Sources are read.
 */
function mediaOffer(sources: SourceDescription[] | null, taking: SourceDescription[]) {
  const acquiring = (among: SourceDescription[], type: string) =>
    among.some((source) => source.asset_types.includes(type));
  return ASSET_TYPE_FAMILIES.map((family) => ({
    value: family.value,
    label: family.label,
    types: family.types
      .filter((type) => sources === null || acquiring(sources, type.value))
      .map((type): OfferedType => {
        const available = sources === null || acquiring(taking, type.value);
        return {
          value: type.value,
          label: type.label,
          available,
          note: available
            ? null
            : acquiring((sources ?? []).filter(usable), type.value)
              ? "Not from these Sources"
              : "Needs a key",
        };
      }),
  })).filter((family) => family.types.length > 0);
}

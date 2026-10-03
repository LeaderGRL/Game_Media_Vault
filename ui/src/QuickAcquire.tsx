import { FormEvent, useState } from "react";

import {
  ASSET_TYPE_FAMILIES,
  AcquisitionRequestDraft,
  buildAcquisitionRequest,
  emptyAcquisitionForm,
} from "./acquisition";

/**
 * The platforms libretro-database publishes a No-Intro or Redump game list for, by the names it
 * gives them, which every game of a platform is read from. Digital catalogs are left out.
 */
export const GAME_LIST_PLATFORMS = [
  "Atari - 2600",
  "Atari - 5200",
  "Atari - 7800",
  "Atari - 8-bit Family",
  "Atari - Jaguar",
  "Atari - Jaguar CD",
  "Atari - Lynx",
  "Atari - ST",
  "Bandai - WonderSwan",
  "Bandai - WonderSwan Color",
  "Coleco - ColecoVision",
  "Commodore - 64",
  "Commodore - Amiga",
  "Commodore - CD32",
  "Commodore - CDTV",
  "GCE - Vectrex",
  "Magnavox - Odyssey2",
  "Mattel - Intellivision",
  "Microsoft - MSX",
  "Microsoft - MSX2",
  "Microsoft - Xbox",
  "Microsoft - Xbox 360",
  "NEC - PC Engine - TurboGrafx 16",
  "NEC - PC Engine CD - TurboGrafx-CD",
  "NEC - PC Engine SuperGrafx",
  "NEC - PC-98",
  "NEC - PC-FX",
  "Nintendo - Family Computer Disk System",
  "Nintendo - Game Boy",
  "Nintendo - Game Boy Advance",
  "Nintendo - Game Boy Color",
  "Nintendo - GameCube",
  "Nintendo - Nintendo 3DS",
  "Nintendo - Nintendo 64",
  "Nintendo - Nintendo 64DD",
  "Nintendo - Nintendo DS",
  "Nintendo - Nintendo DSi",
  "Nintendo - Nintendo Entertainment System",
  "Nintendo - Pokemon Mini",
  "Nintendo - Satellaview",
  "Nintendo - Sufami Turbo",
  "Nintendo - Super Nintendo Entertainment System",
  "Nintendo - Virtual Boy",
  "Nintendo - Wii",
  "Philips - CD-i",
  "SNK - Neo Geo CD",
  "SNK - Neo Geo Pocket",
  "SNK - Neo Geo Pocket Color",
  "Sega - 32X",
  "Sega - Dreamcast",
  "Sega - Game Gear",
  "Sega - Master System - Mark III",
  "Sega - Mega Drive - Genesis",
  "Sega - Mega-CD - Sega CD",
  "Sega - Naomi",
  "Sega - Naomi 2",
  "Sega - PICO",
  "Sega - SG-1000",
  "Sega - Saturn",
  "Sharp - X68000",
  "Sinclair - ZX Spectrum +3",
  "Sony - PlayStation",
  "Sony - PlayStation 2",
  "Sony - PlayStation 3",
  "Sony - PlayStation Portable",
  "Sony - PlayStation Vita",
  "The 3DO Company - 3DO",
  "Tiger - Game.com",
  "Watara - Supervision",
];

interface QuickAcquireProps {
  starting: boolean;
  /** Starts acquiring the request built from the platform chosen. */
  onDownload: (request: AcquisitionRequestDraft) => void;
}

/**
 * The shortest way to acquire: every kind of media of every game of one platform, from every
 * Source. Sources tell media of some regions apart, and none tells languages apart, so a
 * request kept to some of them is a custom one.
 */
export function QuickAcquire({ starting, onDownload }: QuickAcquireProps) {
  const [platform, setPlatform] = useState("");

  function submit(event: FormEvent) {
    event.preventDefault();
    onDownload(
      buildAcquisitionRequest({
        ...emptyAcquisitionForm(),
        autoSources: true,
        platforms: platform,
        assetTypes: ASSET_TYPE_FAMILIES.map((family) => family.value),
      }),
    );
  }

  return (
    <form className="quick-acquire" aria-label="Download everything" onSubmit={submit}>
      <p className="hint">
        Every game of a platform, with every kind of media every Source has for it: covers,
        spines, cartridges, discs, manuals, maps, screenshots, videos and more. Media show in the
        Library as they arrive.
      </p>
      <div className="acquire-targets">
        <label>
          Platform
          <input
            list="game-list-platforms"
            value={platform}
            placeholder="Nintendo - Super Nintendo Entertainment System"
            onChange={(event) => setPlatform(event.target.value)}
          />
          <datalist id="game-list-platforms">
            {GAME_LIST_PLATFORMS.map((name) => (
              <option key={name} value={name} />
            ))}
          </datalist>
        </label>
      </div>
      <button type="submit" disabled={starting || platform.trim() === ""}>
        {starting ? "Starting…" : "Download everything"}
      </button>
    </form>
  );
}

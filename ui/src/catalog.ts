/**
 * The consoles, regions and languages the desktop offers to pick from, so that nothing which
 * must match a game list is typed by hand.
 */

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

/** A console: the platform its game list is named after, its maker and its own name. */
export interface Console {
  platform: string;
  maker: string;
  name: string;
}

/** The name of a platform without its maker, as `Super Nintendo Entertainment System`. */
export function consoleName(platform: string): string {
  const separator = platform.indexOf(" - ");
  return separator < 0 ? platform : platform.slice(separator + 3);
}

function makerOf(platform: string): string {
  const separator = platform.indexOf(" - ");
  return separator < 0 ? "Other" : platform.slice(0, separator);
}

/** Every console a game list is published for. */
export const CONSOLES: Console[] = GAME_LIST_PLATFORMS.map((platform) => ({
  platform,
  maker: makerOf(platform),
  name: consoleName(platform),
}));

/** Consoles of one maker. */
export interface ConsoleGroup {
  maker: string;
  consoles: Console[];
}

/**
 * `consoles` grouped by maker, makers and consoles in name order, keeping those whose maker and
 * name together hold every word of `query`, regardless of case.
 */
export function consolesByMaker(consoles: Console[], query = ""): ConsoleGroup[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const groups = new Map<string, Console[]>();
  for (const console of consoles) {
    const haystack = `${console.maker} ${console.name}`.toLowerCase();
    if (!words.every((word) => haystack.includes(word))) {
      continue;
    }
    groups.set(console.maker, [...(groups.get(console.maker) ?? []), console]);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([maker, members]) => ({
      maker,
      consoles: [...members].sort((a, b) => a.name.localeCompare(b.name)),
    }));
}

/** A value to pick, with the words it shows. */
export interface Option {
  value: string;
  label: string;
}

/** The regions No-Intro and Redump name releases with, the most common first. */
export const COMMON_REGIONS: Option[] = [
  "World",
  "Europe",
  "USA",
  "Japan",
  "France",
  "Germany",
  "Spain",
  "Italy",
  "UK",
  "Netherlands",
  "Sweden",
  "Australia",
  "Canada",
  "Brazil",
  "Korea",
  "China",
  "Asia",
  "Russia",
].map((region) => ({ value: region, label: region }));

/** The other regions No-Intro names releases with, in name order. */
export const OTHER_REGIONS: Option[] = [
  "Argentina",
  "Austria",
  "Bulgaria",
  "Chile",
  "Croatia",
  "Czech",
  "Denmark",
  "Finland",
  "Greece",
  "Hong Kong",
  "Hungary",
  "India",
  "Ireland",
  "Israel",
  "Latin America",
  "Mexico",
  "New Zealand",
  "Norway",
  "Peru",
  "Poland",
  "Portugal",
  "Slovakia",
  "Taiwan",
  "Turkey",
  "United Kingdom",
].map((region) => ({ value: region, label: region }));

/** Every region No-Intro names releases with, the common ones first. */
export const REGIONS: Option[] = [...COMMON_REGIONS, ...OTHER_REGIONS];

/** The languages No-Intro and Redump name releases with, by the code they write. */
export const COMMON_LANGUAGES: Option[] = [
  { value: "En", label: "English" },
  { value: "Fr", label: "French" },
  { value: "De", label: "German" },
  { value: "Es", label: "Spanish" },
  { value: "It", label: "Italian" },
  { value: "Ja", label: "Japanese" },
  { value: "Pt", label: "Portuguese" },
  { value: "Nl", label: "Dutch" },
  { value: "Sv", label: "Swedish" },
  { value: "No", label: "Norwegian" },
  { value: "Da", label: "Danish" },
  { value: "Fi", label: "Finnish" },
  { value: "Pl", label: "Polish" },
  { value: "Ru", label: "Russian" },
  { value: "Zh", label: "Chinese" },
  { value: "Ko", label: "Korean" },
];

/** The other languages No-Intro and Redump name releases with, by name. */
export const OTHER_LANGUAGES: Option[] = [
  { value: "Af", label: "Afrikaans" },
  { value: "Sq", label: "Albanian" },
  { value: "Ar", label: "Arabic" },
  { value: "Eu", label: "Basque" },
  { value: "Bg", label: "Bulgarian" },
  { value: "Ca", label: "Catalan" },
  { value: "Hr", label: "Croatian" },
  { value: "Cs", label: "Czech" },
  { value: "Et", label: "Estonian" },
  { value: "El", label: "Greek" },
  { value: "He", label: "Hebrew" },
  { value: "Hi", label: "Hindi" },
  { value: "Hu", label: "Hungarian" },
  { value: "Is", label: "Icelandic" },
  { value: "Id", label: "Indonesian" },
  { value: "Ga", label: "Irish" },
  { value: "Lv", label: "Latvian" },
  { value: "Lt", label: "Lithuanian" },
  { value: "Ms", label: "Malay" },
  { value: "Fa", label: "Persian" },
  { value: "Ro", label: "Romanian" },
  { value: "Gd", label: "Scottish Gaelic" },
  { value: "Sr", label: "Serbian" },
  { value: "Sk", label: "Slovak" },
  { value: "Sl", label: "Slovenian" },
  { value: "Th", label: "Thai" },
  { value: "Tr", label: "Turkish" },
  { value: "Uk", label: "Ukrainian" },
  { value: "Vi", label: "Vietnamese" },
  { value: "Cy", label: "Welsh" },
];

/** Every language No-Intro and Redump name releases with, the common ones first. */
export const LANGUAGES: Option[] = [...COMMON_LANGUAGES, ...OTHER_LANGUAGES];

/** The name of a language code, or the code itself when it is not offered. */
export function languageName(code: string): string {
  return LANGUAGES.find((language) => language.value === code)?.label ?? code;
}

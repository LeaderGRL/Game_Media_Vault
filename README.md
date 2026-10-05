# Game Media Vault

Game Media Vault acquires, catalogs and preserves video-game media — box art, screenshots,
title screens and more — from several Sources, and links every original to the Release Edition
it belongs to. Originals are kept byte-for-byte in a local, content-addressed vault; uncertain
matches wait for a human decision instead of being guessed.

The same Rust application layer runs behind a Tauri desktop app and a command-line interface.

## What it does today

- **Acquisition Runs** from an Acquisition Request (Sources, platforms, games, regions, Asset
  Types, quality requirements, retention policy), planned across every Source that can serve it
  and explained before they start. Runs are persisted, resumable, pausable and cancellable.
  Sources take turns, so a slow or failing Source never holds back the others. Transient
  failures are retried with backoff, cut downloads resume, and media a Source no longer serves
  completes as unavailable. Requests export to portable documents, and the desktop fills them
  from presets.
- **Sources**:
  - **Libretro Thumbnails**: Box Front, Screenshot and Title Screen images.
  - **LaunchBox Games Database**: box fronts, backs, spines and 3D renders, cartridges, discs,
    screenshots, logos, artwork, flyers, and the marquees, cabinets, control panels and circuit
    boards of arcade games.
  - **SteamGridDB**: community logos, icons and heroes, with the user's own API key.
  - **TheGamesDB**: box fronts and backs, screenshots, title screens, clear logos and fan art,
    with the user's own API key.
  - **ScreenScraper**: regional box scans, spines and 3D boxes, cartridge and disc scans,
    manuals, screenshots, title screens, gameplay videos, logos, fan art and flyers, with
    developer credentials and, optionally, the user's own account.
  - **RAWG**: screenshots and background art of modern and retro games, with the user's own API
    key.
  - **PSX DataCenter**: high-resolution PlayStation cover scans and screenshots, read from its
    public website as a well-behaved client that honors robots.txt and leaves time between
    requests.
  - **VGMaps**: maps of games' levels and worlds, read from its public atlas pages the same
    way.

  API keys stay in the operating system's credential store, never in a vault
  ([ADR 0005](docs/adr/0005-keep-source-api-keys-in-the-os-credential-store.md)). The Sources
  view describes what each Source acquires, what it is known to limit and the failures
  executions recorded, stores or forgets API keys, and enables or disables each Source on this
  machine. [`docs/sources.md`](docs/sources.md) explains how to get each key or credential.
- **Reference catalogs**: a platform's game list downloads by itself from libretro-database
  (`reference sync`), and No-Intro and Redump datafiles and MAME software lists import as Release
  Editions with their identifiers, so acquired media can be matched to known releases; releases
  several catalogs describe with the same dumps, or else the same title, share one Release
  Edition. A record whose evidence points at several editions becomes a Reference Review Item,
  decided in the desktop Review view or with `reference review`, and later imports ask again
  when the evidence changes.
- **Matching and Review**: each candidate is scored against the Library; confident matches link
  automatically, uncertain ones become Review Items to accept, reject or defer.
- **Library**: canonical values derived from every Source's claims, the Preferred Asset of each
  type, coverage of the packaging profiles, search with combinable filters and stable pages;
  gameplay videos (MP4, WebM) play in place, image originals show as they are, and a PDF, such
  as a manual or some maps, shows its first page once its thumbnail is rendered.
- **Export**: every original copies to a folder of your choosing, as
  `<platform>/<game>/<Asset Type>/<file>` with readable, Windows-safe names; exporting again
  copies only what is new.
- **Manuals and documents**: local files of any stored Asset Type, manuals included, import
  unchanged; a PDF original records its page count, title and author, and the Library shows
  them.
- **Derived Assets**: reproducible PNG thumbnails rendered from originals without touching them,
  the first page of a PDF included with pdfium, which release builds ship
  ([ADR 0006](docs/adr/0006-render-pdf-previews-with-pdfium-from-explicit-directories.md)),
  and 3D models (glTF binary) of complete cardboard boxes built from their front, back and spine
  scans, which the desktop previews in 3D.
- **Vault integrity**: verification rehashes every stored object against the catalog and reports
  stale work; repairs apply only the actions named, and never touch an original they cannot
  recover.

See [`docs/SPEC.md`](docs/SPEC.md) for the complete product specification, including what is
planned next.

## Install

Each [release](https://github.com/LeaderGRL/Game_Media_Vault/releases) has an archive per system:

| System | Archive |
| --- | --- |
| Windows (x64) | `game-media-vault-<version>-x86_64-pc-windows-msvc.zip` |
| Linux (x64) | `game-media-vault-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| macOS (Apple Silicon) | `game-media-vault-<version>-aarch64-apple-darwin.tar.gz` |

Extract it anywhere, such as `%LOCALAPPDATA%\Programs\Game Media Vault` on Windows, and keep its
files together: `game-media-vault-desktop` is the desktop app, `game-media-vault` the
command-line interface, and the pdfium library beside them renders PDF previews. Launch
`game-media-vault-desktop`; on Windows, a shortcut to it in
`%APPDATA%\Microsoft\Windows\Start Menu\Programs` adds it to the Start menu.

The binaries are not signed, so Windows SmartScreen asks to confirm the first launch (**More
info**, then **Run anyway**) and macOS opens it once Control-clicked and **Open** chosen. The
desktop app needs WebView2 on Windows, which Windows 11 and an up-to-date Windows 10 include, and
WebKitGTK 4.1 on Linux (`libwebkit2gtk-4.1-0` on Debian and Ubuntu).

The app remembers the vault it opened last outside its folder, so updating means replacing the
extracted files with those of the new release; a vault opens again in a newer version, which
upgrades its catalog.

## Workspace

| Path | Contents |
| --- | --- |
| `crates/domain` | Domain model: requests, releases, assets, matching, coverage |
| `crates/application` | Use cases and the ports they depend on |
| `crates/connectors` | Sources: Libretro Thumbnails, LaunchBox Games Database, SteamGridDB, TheGamesDB, ScreenScraper, RAWG, PSX DataCenter, VGMaps, and the No-Intro, Redump and MAME software list reference catalogs |
| `crates/infrastructure` | SQLite catalog, content-addressed object store, image and PDF transforms, credential store |
| `crates/cli` | The `game-media-vault` command-line interface |
| `src-tauri` | The Tauri desktop shell |
| `ui` | The desktop frontend (React, Vite, Vitest) |

The core never depends on Tauri ([ADR 0001](docs/adr/0001-keep-the-core-independent-from-tauri.md)).

## Prerequisites

- Rust (stable) with `rustfmt` and `clippy`
- Node.js 22
- On Linux, the Tauri system libraries: `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`,
  `librsvg2-dev`, `patchelf`

## Build and run

```bash
npm --prefix ui ci
```

Desktop app in development mode (from `ui/`):

```bash
npm run tauri -- dev
```

In the app:

1. **Open vault**: the vault is created in `Documents/Game Media Vault` unless you choose another
   folder, and opens again next time.
2. **Download**: tick one or more consoles, then pick how many of their games (all, a number or
   a share), the regions, languages and media you want, and how many of each type to keep, and
   press **Start download**. Nothing needs typing.
3. **Activity**: follow each console's search and downloads with progress bars, and the covers
   as they arrive. Consoles download one after another.
4. **Library**: browse your games as covers, filter them, open one to see all its media, and
   **Export…** them to a folder as `<platform>/<game>/<Asset Type>/<file>`.

Sources that need an API key (SteamGridDB, TheGamesDB, ScreenScraper, RAWG) take part once you
store yours in the Sources view.

Debug builds keep line tables only, so building and testing everything takes about 5 GB in
`target/`. Cargo never removes older builds from it, though: `cargo clean` empties it when it
grows, and the next build takes a few minutes.

Command-line interface, with a vault in `./my-vault`:

```bash
cargo run -p game-media-vault-cli -- --vault my-vault reference sync --platform "Nintendo - Nintendo Entertainment System"
```

```bash
cargo run -p game-media-vault-cli -- --vault my-vault acquire --source libretro-thumbnails --platform "Nintendo - Nintendo Entertainment System" --game "Super Mario Bros. (World)" --asset-type box-front
```

```bash
cargo run -p game-media-vault-cli -- --vault my-vault run execute 1
```

Every game of a platform, kept to Japan, with every kind of media the Sources acquire and the
three best of each type and game (`--auto-source` uses every Source whose key you stored):

```bash
cargo run -p game-media-vault-cli -- --vault my-vault acquire --auto-source --platform "Nintendo - Virtual Boy" --region Japan --asset-type any --keep-per-type 3
```

```bash
cargo run -p game-media-vault-cli -- --vault my-vault export --to "D:/Game Media"
```

`cargo run -p game-media-vault-cli -- --help` lists every command: `acquire` and `plan`, `run`
(list, show, start, execute, pause, resume, cancel, export), `review` and `reference review`,
`source` (with `source key set` for API keys), the `import-*` commands for local media and
reference catalogs, `library` and `search`, `derive-thumbnails` and `derive-packaging-models`,
and `verify` and `repair`.

## Tests

The CI runs the same checks:

```bash
npm --prefix ui test
```

```bash
npm --prefix ui run build
```

```bash
cargo fmt --all --check
```

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

```bash
cargo test --workspace --all-features
```

## Documentation

- [`docs/SPEC.md`](docs/SPEC.md) — product specification
- [`docs/sources.md`](docs/sources.md) — what each Source provides and needs
- [`CONTEXT.md`](CONTEXT.md) — domain glossary
- [`docs/adr/`](docs/adr/) — architecture decisions
- [`AGENTS.md`](AGENTS.md) — development workflow (test-first slices, one commit per change)

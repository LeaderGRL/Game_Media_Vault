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

  The Sources view describes what each Source acquires and the failures executions recorded,
  and enables or disables each Source on this machine.
- **Reference catalogs**: No-Intro and Redump datafiles and MAME software lists import as Release
  Editions with their identifiers, so acquired media can be matched to known releases; releases
  several catalogs describe under the same title share one Release Edition.
- **Matching and Review**: each candidate is scored against the Library; confident matches link
  automatically, uncertain ones become Review Items to accept, reject or defer.
- **Library**: canonical values derived from every Source's claims, the Preferred Asset of each
  type, coverage of the packaging profiles, search with combinable filters and stable pages.
- **Derived Assets**: reproducible PNG thumbnails rendered from originals without touching them,
  and 3D models (glTF binary) of complete cardboard boxes built from their front, back and spine
  scans, which the desktop previews in 3D.
- **Vault integrity**: verification rehashes every stored object against the catalog and reports
  stale work; repairs apply only the actions named, and never touch an original they cannot
  recover.

See [`docs/SPEC.md`](docs/SPEC.md) for the complete product specification, including what is
planned next.

## Workspace

| Path | Contents |
| --- | --- |
| `crates/domain` | Domain model: requests, releases, assets, matching, coverage |
| `crates/application` | Use cases and the ports they depend on |
| `crates/connectors` | Sources: Libretro Thumbnails, LaunchBox Games Database, and the No-Intro, Redump and MAME software list reference catalogs |
| `crates/infrastructure` | SQLite catalog, content-addressed object store, image transforms |
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

Command-line interface, with a vault in `./my-vault`:

```bash
cargo run -p game-media-vault-cli -- --vault my-vault import-no-intro --file nes.dat --max-games 500
```

```bash
cargo run -p game-media-vault-cli -- --vault my-vault acquire --source libretro-thumbnails --platform "Nintendo - Nintendo Entertainment System" --game "Super Mario Bros. (World)" --asset-type box-front
```

```bash
cargo run -p game-media-vault-cli -- --vault my-vault run execute 1
```

`cargo run -p game-media-vault-cli -- --help` lists every command: `acquire` and `plan`, `run`
(list, show, start, execute, pause, resume, cancel, export), `review`, `source`, the
`import-*` commands for local box fronts and reference catalogs, `library` and `search`,
`derive-thumbnails` and `derive-packaging-models`, and `verify` and `repair`.

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

# Game Media Vault

Game Media Vault acquires, catalogs and preserves video-game media — box art, screenshots,
title screens and more — from several Sources, and links every original to the Release Edition
it belongs to. Originals are kept byte-for-byte in a local, content-addressed vault; uncertain
matches wait for a human decision instead of being guessed.

The same Rust application layer runs behind a Tauri desktop app and a command-line interface.

## What it does today

- **Acquisition Runs** from an Acquisition Request (Sources, platforms, games, regions, Asset
  Types, quality requirements, retention policy). Runs are persisted, resumable, pausable and
  cancellable; each Source is discovered once per run.
- **Libretro Thumbnails** connector: Box Front, Screenshot and Title Screen images from the
  `libretro-thumbnails` repositories.
- **Reference catalogs**: No-Intro datafiles import as Release Editions with their identifiers,
  so acquired media can be matched to known releases.
- **Matching and Review**: each candidate is scored against the Library; confident matches link
  automatically, uncertain ones become Review Items to accept, reject or defer.
- **Library**: canonical values derived from every Source's claims, the Preferred Asset of each
  type, coverage of the packaging profiles, search with combinable filters and stable pages.
- **Derived Assets**: reproducible PNG thumbnails rendered from originals without touching them.

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

`cargo run -p game-media-vault-cli -- --help` lists every command (runs, reviews, imports,
Library search, thumbnails).

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
- [`CONTEXT.md`](CONTEXT.md) — domain glossary
- [`docs/adr/`](docs/adr/) — architecture decisions
- [`AGENTS.md`](AGENTS.md) — development workflow (test-first slices, one commit per change)

# Game Media Vault — Product and Technical Specification

## 1. Purpose

Game Media Vault is a Rust-based acquisition and cataloging engine with a Tauri desktop interface and a CLI. Its purpose is to discover, download, preserve, normalize, compare, and browse video-game media across as many platforms, regions, languages, editions, and source providers as possible.

The application is designed first as a private research and production tool. It must preserve enough provenance and licensing metadata that later commercial-use decisions can be made per source and per asset.

## 2. Product Goals

- Collect media for specific games or broad platform catalogs.
- Distinguish collectible Release Editions instead of collapsing all media under a title.
- Support APIs, public HTML pages, downloadable datasets, repositories, and local imports through one connector abstraction.
- Let the user select exactly which asset types, sources, platforms, languages, regions, games, and quality requirements are wanted.
- Preserve original files byte-for-byte and generate derivatives separately.
- Resume long-running acquisition jobs safely after crashes or restarts.
- Retain provenance, source assertions, confidence, and conflicts.
- Expose incomplete and uncertain data clearly instead of silently guessing.
- Run the same acquisition engine from Tauri or the CLI.

## 3. Explicit Boundaries

- At least one Source option must be selected for every Acquisition Request. `Auto` may be offered as an explicit selectable source strategy; an empty source selection is invalid.
- Connectors may use documented APIs, unofficial/public APIs, downloadable indexes, repositories, public HTML extraction, and user-provided local datasets.
- Public web connectors must respect technical access boundaries. The product will not contain mechanisms whose purpose is to bypass authentication, CAPTCHA, paywalls, access controls, or anti-bot challenges.
- Downloading an Asset does not imply redistribution rights. Provenance and source policy metadata must remain attached to the Asset.
- Original files are immutable after successful ingestion.

## 4. Primary User Flows

### 4.1 Create an Acquisition Request

The user can select:

- one or more Sources, or an explicit `Auto` source strategy;
- one or more Platforms;
- all games, an explicit game list, a query result, or a maximum game count;
- zero or more Regions;
- zero or more Languages;
- one or more Asset Types;
- optional quality constraints;
- a Retention Policy;
- optional download and concurrency limits.

Every filter is independent. A request containing only `Packaging > Front` must not intentionally download unrelated asset types.

### 4.2 Monitor an Acquisition Run

The run view shows:

- current phase;
- discovered games and Release Editions;
- candidates found by source;
- accepted, rejected, skipped, and failed candidates;
- bytes downloaded;
- current throughput;
- source quota/rate-limit state;
- retry queue;
- Review Items created;
- coverage gained by the run.

Runs can be paused, resumed, cancelled, and resumed after application restart.

### 4.3 Browse the Library

The library is centered on Games and Release Editions. A Release Edition view exposes:

- canonical metadata and source assertions;
- region, languages, product codes, revision, packaging, publisher, and dates;
- all original Assets grouped by Asset Type;
- Preferred Assets;
- Derived Assets;
- provenance and source URLs;
- quality metadata;
- Coverage Status;
- unresolved Review Items.

The library must support explicit filters for `Complete`, `Partial`, `Needs review`, platform, region, language, source, and Asset Type.

## 5. Asset Taxonomy

The initial taxonomy is hierarchical and individually selectable.

### Packaging

- Box Front
- Box Back
- Spine
- Inner Cover
- Box Texture
- Box 3D Render
- Box 3D Model
- Slipcover / Sleeve
- Insert

### Physical Media

- Cartridge
- Cartridge Front
- Cartridge Back
- Cartridge Label
- Disc
- Disc Front
- Disc Back
- Disc Label
- PCB
- Cassette / Tape
- Floppy Disk

### Documentation

- Manual
- Manual Page
- Strategy Guide
- Map
- Reference Card
- Registration Card
- Warranty / Safety Insert

### Digital Media

- Screenshot
- Title Screen
- Gameplay Video
- Trailer
- Logo
- Icon
- Wallpaper / Artwork

### Promotional and Historical

- Flyer
- Advertisement
- Poster
- Promotional Artwork
- Press Material
- Magazine Scan

### Hardware / Arcade

- Arcade Cabinet
- Control Panel
- Marquee
- Bezel
- Controller
- Accessory

### Other

- Soundtrack
- Texture
- 3D Model
- Other

The taxonomy must be extensible without a database migration for every newly discovered source-specific label. Source labels map onto canonical Asset Types while the original label remains stored.

## 6. Acquisition Request Model

Conceptually:

```rust
pub struct AcquisitionRequest {
    pub sources: SourceSelection,
    pub platforms: Vec<PlatformSelector>,
    pub games: GameSelection,
    pub regions: Vec<RegionSelector>,
    pub languages: Vec<LanguageSelector>,
    pub asset_types: Vec<AssetTypeSelector>,
    pub quality: Option<QualityRequirements>,
    pub retention: RetentionPolicy,
    pub limits: AcquisitionLimits,
}
```

Validation rules:

- `sources` must resolve to at least one Source.
- `platforms` must resolve to at least one Platform unless the selected game set already fixes platforms explicitly.
- `asset_types` must contain at least one Asset Type.
- empty `regions` means any region.
- empty `languages` means any language.
- quality requirements are optional.

## 7. Quality Filtering

Quality requirements can independently constrain:

- minimum width and height;
- minimum longest edge;
- minimum pixel count;
- original-only assets;
- accepted MIME/container types;
- maximum compression or minimum bitrate where measurable;
- preferred scan type;
- preferred source priority;
- best-available mode.

The engine should not discard a candidate solely because another candidate is better until the configured Retention Policy has been applied.

## 8. Retention Policies

### Keep Everything

All accepted non-identical Assets are retained. Exact byte duplicates share one stored object but preserve every source relationship.

### Keep Best Per Type

The best accepted Asset for the matching Release Edition and Asset Type becomes the retained/preferred media according to the configured scoring policy. Discovery metadata for rejected alternatives remains available so later re-acquisition or policy changes are possible.

Preference scoring is explainable. The UI must be able to show why one candidate outranked another.

## 9. Release Edition Identity

A Release Edition is distinct when one or more materially collectible properties differ, including:

- Platform;
- territory/market;
- product or serial code;
- commercial edition;
- packaging format;
- revision;
- bundle membership;
- publisher/distributor variant;
- meaningful printed-language variant;
- known reissue or print run when identifiable.

No single provider is treated as absolute truth. Source-specific claims are stored as Release Assertions. Canonical values are derived with provenance and confidence.

## 10. Matching and Confidence

Matching can use:

- normalized title;
- alternate titles;
- platform identifiers;
- product/serial codes;
- region;
- language;
- publisher;
- release date;
- packaging descriptors;
- ROM/disc hashes where catalog sources legitimately provide them;
- source IDs and cross-references.

Every non-trivial match carries evidence and a confidence score. Thresholds are configuration, not hard-coded domain constants.

Suggested behavior:

- high confidence: auto-link;
- medium confidence: create a Review Item and keep the candidate staged;
- low confidence: do not attach automatically.

Human decisions are persisted and can become future matching evidence.

## 11. Coverage

Coverage is evaluated through configurable Coverage Profiles rather than a single hard-coded percentage.

Initial profiles:

- `Packaging`: enough assets to reconstruct the external package for that packaging type;
- `Physical`: packaging plus the primary physical media and expected physical inserts;
- `Archival`: all known/required collectible media categories for the edition.

Initial status labels:

- `Partial`;
- `Packaging Complete`;
- `Physical Complete`;
- `Archival Complete`.

Requirements vary by packaging family. A PlayStation jewel case, Nintendo DS case, SNES cardboard box, arcade board, floppy release, and digital-only release must not share one universal checklist.

Reference catalogs can also be used to measure catalog coverage, for example known Release Editions versus discovered Release Editions for a platform/territory.

## 12. Connector Architecture

Each Source is implemented behind a connector capability contract. A connector declares what it supports instead of pretending every source has identical features.

Possible capabilities include:

- platform enumeration;
- game search;
- release enumeration;
- release lookup;
- metadata discovery;
- media discovery by Asset Type;
- direct media download;
- pagination;
- locale/region filtering;
- resumable transfer;
- quota reporting;
- authentication requirements.

Connector families:

- API connector;
- public web connector;
- downloadable dataset connector;
- repository connector;
- local import connector.

A single Source may expose multiple connector implementations when useful, such as API plus public dataset.

## 13. Source Registry

The registry stores source capabilities and policy metadata separately from connector code.

Candidate initial providers include:

- ScreenScraper;
- GameTDB;
- EmuMovies;
- LaunchBox Games Database;
- MobyGames;
- TheGamesDB;
- Libretro Thumbnails;
- GamesDatabase;
- Gaming Alexandria;
- Sega Retro;
- ReplacementDocs;
- RetroMags;
- PSX DataCenter and platform-specific reference sites;
- VGMaps;
- RAWG;
- SteamGridDB.

Candidate reference catalogs include:

- No-Intro DATs;
- Redump DATs;
- MAME Software Lists.

This list is a registry seed, not a closed allowlist.

## 14. Acquisition Pipeline

```text
Acquisition Request
        |
        v
Request validation
        |
        v
Source planning + capability filtering
        |
        v
Catalog discovery
        |
        v
Release normalization
        |
        v
Release matching / Review Items
        |
        v
Asset candidate discovery
        |
        v
Candidate scoring + policy filtering
        |
        v
Persistent download queue
        |
        v
Verification + hashing
        |
        +-----------------------+
        |                       |
        v                       v
Immutable object store      Metadata catalog
        |
        v
Derived-asset workers
        |
        v
Coverage evaluation
```

Every stage must be restartable from persisted state. Connector failures must not corrupt accepted assets or erase progress from unrelated sources.

## 15. Storage Architecture

### Metadata

SQLite is the initial local catalog because the primary product is a single-user desktop/CLI tool. Database access must stay behind repository interfaces so a future server deployment can migrate to PostgreSQL without leaking SQL concerns into the domain.

### Media Objects

Original binary data lives outside SQLite in a content-addressed object store keyed by BLAKE3.

Example layout:

```text
vault/
  objects/
    ab/
      cd/
        abcdef...     # immutable original bytes
  derived/
    ...
  staging/
    ...
```

The database records MIME type, byte length, dimensions/duration where applicable, original filename, source relationships, integrity information, and object hash.

Identical files from multiple Sources reuse one physical object while retaining each provenance record.

## 16. Derived Assets

Derivatives are reproducible outputs associated with their original Asset and a transformation recipe.

Initial transformations may include:

- crop;
- resize;
- format conversion;
- image normalization;
- PDF page extraction;
- thumbnail generation;
- texture preparation;
- generated box geometry/3D representation.

A transformation must never mutate the original object.

## 17. 3D Packaging

Game Media Vault should support both downloaded 3D assets and generated 3D packaging.

Generated packaging uses packaging-family templates with known geometry and expected texture slots. Examples include jewel cases, DVD cases, DS/3DS cases, cardboard boxes, clamshell cases, cartridges, optical discs, and floppy packaging.

The `Packaging` Coverage Profile determines whether enough accepted assets exist to build a usable generated model.

## 18. Persistent Work Queue

Acquisition is modeled as persisted work units rather than ephemeral UI tasks.

Required characteristics:

- bounded concurrency globally and per Source;
- source-specific rate limiting;
- exponential backoff with jitter for transient failures;
- retry ceilings configurable per error family;
- explicit permanent failures;
- crash-safe state transitions;
- resumable downloads when the server supports them;
- idempotent processing;
- cancellation at sensible boundaries;
- pause/resume without losing discovery results.

The scheduler should favor useful progress across Sources instead of allowing one slow or throttled provider to block an entire run.

## 19. Rust Workspace Shape

The intended workspace boundary is:

```text
crates/
  domain/          # Pure domain types, rules, matching decisions
  application/     # Use cases and orchestration
  catalog/         # Persistence interfaces and SQLite implementation
  storage/         # Object store and derived asset storage
  acquisition/     # Planning, scheduling, download pipeline
  connectors/      # Connector traits and concrete source adapters
  media/           # Inspection, hashing, transforms
  cli/             # CLI frontend
src-tauri/          # Thin Tauri command/event adapter
ui/                 # Desktop frontend
```

Exact crate boundaries can be adjusted during implementation when cohesion becomes clearer. The invariant is that the core acquisition/application logic has no dependency on Tauri.

## 20. Tauri UI

Primary navigation:

- `Acquire`;
- `Runs`;
- `Library`;
- `Review`;
- `Sources`;
- `Settings`.

### Acquire

A guided but compact request builder with hierarchical multi-select controls for Sources, Platforms, Asset Types, Regions, and Languages. Presets can populate these selections but all resulting values remain editable.

Initial preset examples:

- `3D Box Builder`;
- `Archival`;
- `Frontend Emulator`;
- `Manuals Only`;
- user-defined presets.

### Review

Review Items show the competing Release Editions or metadata values, thumbnails/previews, evidence from each Source, matching score breakdown, and actions to accept, reject, merge, split, or defer.

### Sources

Each Source displays:

- enabled/disabled state;
- available acquisition methods;
- authentication state when applicable;
- supported Asset Types;
- supported Platforms where known;
- current quotas/rate limits where discoverable;
- recent success/error statistics.

## 21. CLI

The CLI is a first-class frontend over the same application layer.

Expected command families:

```text
game-media-vault acquire ...
game-media-vault run list
game-media-vault run resume <id>
game-media-vault library ...
game-media-vault review ...
game-media-vault import ...
game-media-vault verify ...
game-media-vault source ...
```

Requests should also be serializable to a declarative file so large acquisitions can be reproduced on another machine.

## 22. Performance Principles

- Streaming downloads; avoid loading large media into memory unnecessarily.
- Hash while streaming when practical to avoid an extra full-file pass.
- Bounded channels/queues to provide backpressure.
- Batched database writes where durability semantics permit it.
- Explicit indexes for source IDs, release identifiers, hashes, review state, and library filtering.
- Lazy media decoding; inspect headers/metadata before full decode where possible.
- Concurrent independent source work under per-source limits.
- No UI dependency in background workers.
- Benchmark matching, catalog lookup, hashing, and high-volume import paths before micro-optimizing them.

## 23. Reliability and Observability

Every Acquisition Run records structured statistics and failure reasons.

Logs should include stable run/work-item/source identifiers while avoiding secrets and authentication tokens. The UI should expose actionable failures rather than raw log-only errors.

The application should be able to verify the vault by checking:

- database references to object-store files;
- object hashes;
- missing/corrupt originals;
- orphaned derived assets;
- interrupted staging files;
- stale work items.

## 24. Configuration and Secrets

Source credentials and API tokens are configuration, never catalog metadata. Secrets must use OS-appropriate protected storage when available and must not be written into exported Acquisition Requests, logs, or source provenance records.

## 25. Implementation Sequence

### Milestone 1 — Vertical Slice

- Rust workspace and domain model;
- SQLite catalog;
- content-addressed object store;
- persisted Acquisition Request / Run;
- connector contract;
- one simple provider connector;
- one reference-catalog importer;
- download + BLAKE3 verification;
- minimal CLI;
- minimal Tauri Acquire/Run/Library flow.

### Milestone 2 — Matching and Review

- Release Assertions;
- confidence scoring;
- Review Items;
- canonicalization;
- duplicate handling;
- Preferred Asset selection.

### Milestone 3 — Coverage and Media Processing

- Coverage Profiles;
- derived images;
- PDF/manual handling;
- packaging templates;
- 3D generation pipeline.

### Milestone 4 — Connector Expansion

- API connectors;
- public web connectors;
- downloadable/repository connectors;
- source health and capability UI;
- source-specific rate-limit strategies.

### Milestone 5 — Large-Scale Operation

- multi-day acquisition hardening;
- NAS/server-oriented CLI workflows;
- export/import of requests and catalog snapshots;
- profiling and throughput optimization;
- recovery and integrity tooling.

## 26. Acceptance Criteria for the First Usable Version

The first usable version is complete when a user can:

1. choose at least one Source, one Platform, and one or more Asset Types;
2. target a bounded set of games;
3. run acquisition through the persisted engine;
4. stop and resume the run without losing completed work;
5. store original files unchanged in the content-addressed vault;
6. browse Game -> Release Edition -> Asset in Tauri;
7. see provenance for every accepted Asset;
8. see Partial versus complete packaging coverage;
9. route uncertain edition matches to Review instead of silently accepting them;
10. execute the same core workflow from the CLI.

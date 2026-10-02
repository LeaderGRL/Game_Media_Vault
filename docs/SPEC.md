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

The desktop Acquire view fills requests from presets. Built-ins set the Asset Types and retention of a purpose — 3D Box Builder (box front, back and spine), Archival (the Packaging, Physical Media and Documentation families), Frontend Emulator (box front, screenshot, title screen and logo) and Manuals Only (manuals) — and leave the other fields, and everything they set, editable; the request they produce is validated like any other. Custom presets save the whole request under a name, can be updated by saving again, renamed and deleted, and survive restarts in the webview's local storage; they cannot take a built-in's name.

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

Every frontend searches the library through one use case. A search combines a title text (matched case-insensitively against the game title and its canonical title), platforms, regions, Sources (of an Asset's provenance or of an assertion), Asset Types or families of retained Assets, and statuses: `Complete` (coverage at least Packaging Complete), `Partial` (coverage evaluated and still Partial) and `Needs review` (an undecided Review Item offers the Release Edition). Values of one filter widen the search; different filters narrow it. Results are ordered by title, platform, region and edition, then Release Edition, and come in pages (50 by default, at most 500) with the total match count; each page reports the newest Release Edition it considered (`as_of`), and a later page given the cursor of the previous one and that `as_of` searches the same Release Editions in the same order: Release Editions are never deleted and new ones receive larger ids, so releases imported in between neither repeat nor shift later pages. Filters are evaluated against the current state of each release, so a release whose Assets, coverage or Review Items change between two pages can enter or leave the results of the later page, which reports the total of that moment; the library is live data, not a frozen snapshot. Release Editions do not record languages yet, so language filtering waits for reference data that provides them.

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

The catalog stores the Asset Types some connector can acquire: Box Front, Box Back, Spine, Box 3D Render, Cartridge Front, Cartridge Back, Disc, PCB, Screenshot, Title Screen, Logo, Wallpaper / Artwork, Flyer, Arcade Cabinet, Control Panel and Marquee so far. Libretro Thumbnails provides Box Front, Screenshot and Title Screen from its `Named_Boxarts`, `Named_Snaps` and `Named_Titles` folders, recorded as the source label; a run discovers only the folders of the types it selects. LaunchBox Games Database provides all of them from its daily `Metadata.zip` dataset, as the `Box - Front`, `Box - Back`, `Box - Spine`, `Box - 3D`, `Cart - Front`, `Cart - Back`, `Disc`, `Arcade - Circuit Board`, `Screenshot - Gameplay`, `Screenshot - Game Title`, `Clear Logo`, `Fanart - Background`, `Advertisement Flyer - Front`, `Arcade - Cabinet`, `Arcade - Control Panel` and `Arcade - Marquee` image types, recorded as the source label (MAME and FBNeo releases are looked up under its `Arcade` platform); image types it maps to no stored Asset Type are never fetched. A selector for a type no connector acquires yet, or for a family that includes one, is refused before discovery. The desktop Library groups each release's originals by Asset Type in taxonomy order.

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

Requirements are measured on the stored original (its media type and pixel size, read from its bytes): minimum width, height, longest edge and pixel count, and accepted media types. Every acquired Asset is an original, so `original_only` always holds. An original that falls short is not linked; its work completes with the shortfalls recorded (shown as "below quality" run counts) and its object stays unreferenced until vault verification. The match still settles the candidate: an undecided Review Item is closed as `auto_resolved`, the work parked on it in other runs is requeued so each run applies its own requirements, and links of the candidate to other Release Editions are removed. Requirements the engine cannot measure or apply yet (compression ratio, bitrate, and the scan type, source priority and best-available preferences, which need a configurable scoring policy) are refused before discovery.

## 8. Retention Policies

### Keep Everything

All accepted non-identical Assets are retained. Exact byte duplicates share one stored object but preserve every source relationship.

### Keep Best Per Type

The best accepted Asset for the matching Release Edition and Asset Type becomes the retained/preferred media according to the configured scoring policy. Discovery metadata for rejected alternatives remains available so later re-acquisition or policy changes are possible.

Preference scoring is explainable. The UI must be able to show why one candidate outranked another.

The Preferred Asset of each Release Edition and Asset Type is derived from its retained Assets whenever the library is read: the original with the most pixels wins (an unknown pixel size ranks below every known one), then the one with more bytes at the same pixel count, which usually means less compression, then the one acquired first. Each other Asset of the type carries the reason it ranks lower.

A Keep Best Per Type run links an acquired original only if it would become the Preferred Asset, comparing it with the retained Assets in the same transaction as the link. An outranked original is not linked: its work completes with the outranking Asset and the reason recorded (shown as "outranked" run counts), and the candidate is settled as for a below-quality original. Bytes a retained Asset already holds add provenance to it. Keep Best Per Type never detaches Assets other runs or imports retained; they only stop being preferred (ADR 0004).

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

No single provider is treated as absolute truth. Source-specific claims are stored as Release Assertions. Canonical values are derived with provenance and confidence when the library is read, so assertions are never rewritten. For each single-valued field (title, region, revision), every Source takes part with its most recently observed claim (re-importing a claim makes it the latest again); the value most Sources agree on (ignoring case and spacing) is selected, a tie keeps the value claimed first, and the confidence is the share of Sources that agree. The contributing and conflicting claims are kept with the selection. Identifiers are multi-valued and remain plain assertions.

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

The coverage of a Release Edition is derived whenever the library is read. Its Packaging Family comes from its platform name (as No-Intro, Redump and Libretro name platforms; catalogs whose name ends in `(Digital)` or `(PSN)` are digital-only); a platform whose family is not known yet is not evaluated rather than held to a checklist that does not fit it. Each profile adds Asset Types to the previous one:

| Packaging Family | Packaging | Physical adds | Archival adds |
| --- | --- | --- | --- |
| Cardboard box (NES, SNES, N64, Game Boy, Atari) | box front, box back, spine | cartridge, manual | insert |
| Jewel case (PlayStation, Saturn, Mega-CD, Dreamcast) | box front, box back, spine | disc, manual | insert |
| Keep case (PlayStation 2–4, PSP, GameCube, Wii, Xbox) | box front, box back, spine | disc | manual, insert |
| Cartridge case (DS, 3DS, Switch, Vita) | box front, box back, spine | cartridge | manual, insert |
| Arcade board (MAME, FBNeo) | marquee | PCB | flyer, control panel, bezel |
| Digital only | box front | — (no Physical profile) | logo, icon |

The Coverage Status is the last profile, in that order, whose requirements and those of every previous profile are all retained; otherwise it is `Partial`. A retained Cartridge Front meets a cartridge requirement, since it shows the cartridge itself; a Cartridge Back alone does not. Every profile exposes the Asset Types it still misses.

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

LaunchBox Games Database is a downloadable dataset rather than a repository: discovery downloads its `Metadata.zip` once, reads the game records of the requested platforms (all of them, or those whose title matches a requested release regardless of case, spacing, subtitle separator and article placement, so "Legend of Zelda, The - A Link to the Past" matches "The Legend of Zelda: A Link to the Past") and then their images, served by `images.launchbox-app.com`. Unlike Libretro Thumbnails, it covers whole platforms and records the region of each image (named as No-Intro does: North America and United States are USA, United Kingdom is UK, The Netherlands is Netherlands), so region filters and all-games selections are honoured; language filters and platforms it has no name for are refused when planning, without downloading the dataset. Records missing an identifier, a file name or a type, or whose file name could address anything but an image, are skipped. Candidates keep the title the request names, or, when every game is requested, the LaunchBox title as No-Intro writes it (a leading article moved after the main title, subtitles after " - "), so they match the releases the Library imported.

Connectors own their transport credentials. Discovered candidates carry a stable, credential-free absolute locator that is persisted with Review Items, work items and provenance; a connector adds API keys, sessions or signed URLs only inside its download step. Candidates whose locator is not an absolute URL free of userinfo, query and fragment are rejected before anything is persisted; connectors that address media with request parameters expose a synthetic, path-based locator instead.

Before a run is persisted, from the desktop or the CLI `acquire` command, its Acquisition Plan is made with the registered connectors (one per Source: Libretro Thumbnails and LaunchBox Games Database), and each connector checks the plan against what its Source can satisfy and may consult the Source to do so: Libretro Thumbnails refuses unbounded game selections, region and language filters (it provides no such evidence) and platforms it declares no repository for. A refused plan is `unsupported`; a Source that cannot be reached for the check fails the start without creating a run. Execution repeats the check only until the Source's discovery is recorded: resuming a discovered run never consults the Source for its plan.

An Acquisition Plan spreads a request over Sources. An explicit selection contacts only the selected Sources, each once however often it is selected, and each needs a registered connector (otherwise the plan is `unsupported`); `Auto` considers every registered connector, so the user chooses it deliberately and never gets it from an empty selection. Each Source is planned for the requested Asset Types its connector acquires, once the connector accepts the request (which may consult the Source and may refuse platforms or filters it cannot serve). A Source that cannot download media or acquires none of the requested types is left out without being consulted; every left-out Source is listed with its reason. Requirements the engine cannot apply yet and acquisition limits are refused before any Source is consulted. Every requested selector must be acquired by a planned Source (a family never is, as for single-Source runs): a selector the capabilities of the selected connectors leave uncovered is refused without consulting any Source, and one left uncovered by Sources that refused the request is refused with the reason each Source was left out; both refusals are `unsupported`. The plan names, for each requested selector, the planned Sources that acquire it. `game-media-vault plan` takes the arguments of `acquire` and prints the plan, made with the same registered connectors, as JSON without starting a run or touching the vault. The desktop Acquire view's "Check plan" action shows, for each requested Asset Type, the Sources that acquire it and why the others are left out, and forgets the plan as soon as the request changes. A run records the Sources its plan kept when it started and executes those alone, each with its own connector: a Source left out then, or registered later, is never contacted for it. Each Source is discovered once per run and independently, and a pause stops further discoveries, even one that lands while a discovery fails. A planned Source not yet discovered checks its part of the plan again (the plan as a whole was checked when the run started): one whose connector is no longer registered, whose capabilities no longer serve the request, that now refuses it, cannot be reached or fails to discover leaves the work of the others to run, keeps the run running and fails the execution once that work is done, and a later execution discovers only the Sources still undiscovered. Likewise, a Source whose download fails, before or partway through its body, keeps its remaining work queued for a later execution while the work of the other Sources runs. Every HTTP request of a connector retries transient failures (a connection that fails or times out, HTTP 429 or a 5xx answer, but never a redirect loop) up to four attempts in all, waiting before each retry for an exponential backoff that doubles from half a second up to eight seconds, jittered between half the backoff and all of it and at least the `Retry-After` the Source asked for (in seconds or as an HTTP date) within that bound; any other refusal fails at once, and a request still failing after its last attempt fails as before, naming how many attempts it made. A download whose connection drops partway resumes from the bytes already received, within the attempts the first request left, from the resource it was redirected to, when that resource serves byte ranges (`Accept-Ranges: bytes`, regardless of case) and a strong ETag lets `If-Range` prove the media unchanged; the Source must answer with the whole rest of the media (`206` with a `Content-Range` from the received bytes to the end), and a body that ends before the media does counts as cut short. Otherwise, as with a weak ETag or only a modification date, a redirect elsewhere or other media, it fails as before, never splicing two versions. An execution lets the planned Sources take turns: each round processes the oldest queued work of every Source that still has some, so a Source with a long queue never holds back the others; downloads still run one at a time, so a slow or retried download delays the next work of every Source. A download the Source answers with HTTP 404 or 410 is permanent rather than retryable: its work completes as unavailable with the reason, counted on the run (and shown by the desktop Runs view), and the Source goes on with its other work. Runs of vaults upgraded from schema version 6 plan the Sources their request selects explicitly. A run completes once every planned Source is discovered and no work remains: an execution that sees the run resumed after a pause stopped its discoveries discovers the Sources left before completing it.

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

No-Intro and Redump datafiles (Logiqx XML) import through the same reader, as `game-media-vault import-no-intro` and `import-redump`: the header names the platform, and each game entry becomes a Release Edition asserted by its Source, with its title, region and revision, a `source_record` identifier that keeps re-imports idempotent, and the name and checksums of every ROM or disc track. Redump disc tags such as `(Disc 1)` become the edition. When the header records a version, each release it asserts also carries a `dat_version` identifier, so the edition of the datafile that asserted it stays identifiable. A game entry too malformed to read (without a name, or with a ROM or track whose attributes cannot be read) is skipped without invalidating the others, and the import summary counts it as a `skipped_records`; a datafile that is not well-formed XML, or ends before its elements close, is invalid as a whole, even where it breaks past the releases an import reads. A self-closing game entry names a release without dumps, or is skipped without a name. Datafiles describe dumps and never contain playable content.

MAME software lists (`hash/<system>.xml`) import as `game-media-vault import-mame-software-list`: the list name gives the platform as No-Intro and Redump name it (`nes` is `Nintendo - Nintendo Entertainment System`; a list this importer does not name keeps its own description, else its name, and a list without a name is invalid, since its name keeps the identities of its records apart), and each software becomes a Release Edition asserted by `mame-software-lists`. Its description gives the title, the regions of its first tag in No-Intro spelling (`Euro` is Europe, `Jpn` is Japan, and spelled-out names such as Portugal stay as they are) and its revision (`Rev. A` is `Rev A`; `v1.1` and `Version 2.0` are revisions too); its list, short name and parent (`clone_of`), year, publisher, serial, barcode, release date, alternate title, developer, version and language are kept as identifiers, with the name, CRC and SHA-1 of every ROM or disk, and a `source_record` identifier keeps re-imports idempotent. The lists record no version of their own, so `--mame-version` names the MAME release they came with, asserted as `mame_version`. Entries without a short name or a description are skipped and counted, like malformed datafile entries, blank metadata values are left out, and a list that ends before its elements close is invalid.

A reference record that another source already describes joins that Source's Release Edition instead of creating another, so Canonical Values weigh both sources. The evidence is the title: the one edition another source titled the same (regardless of case), on the same platform, region and edition, links the record, and the importing source then asserts a `linked_by` identifier naming that evidence (`title`), which moves with its catalog like its other claims. When several editions carry that title, none is linked. Dump checksums link nothing yet: the catalog keeps every claim a source ever made, so it cannot tell which dumps one edition of a catalog asserts together, nor how many releases share a checksum, until dump evidence is recorded per observation. A record of another region or edition joins the Game a source titled the same on that platform, while a record left unlinked never joins an edition another source holds, even through a Game its own source titled. Re-imports stay idempotent through each source's own `source_record`; records imported before linking existed are not reconciled. Uncertain cross-source matches are not routed to Review yet.

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

Catalog files are identified by a Game Media Vault SQLite application id and versioned with `user_version`. Opening a vault refuses foreign databases and catalogs written by a newer version without modifying them; older supported versions are upgraded through ordered, transactional migrations.

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

The database records MIME type, byte length, dimensions/duration where applicable, original filename, source relationships, integrity information, and object hash. The object store reads the media type and pixel size of each original while storing it (from its first bytes, from a JPEG frame header wherever metadata segments put it, from the first TIFF image file directory wherever the header points, from HEIF item properties and a JPEG XL codestream wherever other boxes put them, and from a portable anymap header through comments of any length), so they describe the bytes rather than a file extension. Only these formats, whose headers metadata commonly pushes past the first 256 KiB, are followed while the original streams; another image whose header exceeds that prefix keeps its media type with an unknown pixel size, which falls short of every size requirement. Raster image formats get their media type, as do PDF documents and glTF binary models (`model/gltf-binary`); GPU texture containers, layered editing documents and originals whose format is unknown are kept as `application/octet-stream`. Assets recorded before media inspection (vaults upgraded from schema version 2) keep unknown media until vault verification inspects their objects again; meanwhile the desktop Library decides their thumbnails from the file name, with the same image types. An image the webview cannot show is replaced by a "Preview unavailable" placeholder.

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

A recipe identifies its transformation and parameters (`thumbnail-png-256`, say). Its output for an original is generated once and recorded under the original's hash and the recipe, so Assets sharing identical bytes share their Derived Assets and running the same recipe again reuses what exists. Outputs are content-addressed like originals but stored apart, under `derived/`; originals are only read. A transformation that fails is reported and the others still run; originals the transformer cannot read are skipped. Assets sharing an original may record different media types (one left unidentified by a schema-version-2 vault, say): the original is read as the first of them the transformer can decode. Every library Asset lists the Derived Assets of its original.

The first recipe renders thumbnails: a raster original (PNG, JPEG, GIF, WebP, BMP, TIFF, ICO, PNM, QOI or TGA) is scaled down to fit its longest edge within the bound, keeping its aspect ratio, and encoded as PNG; smaller images keep their size. Each original is decoded as the format its media type records, so formats without a signature, such as TGA, decode too. An original larger than 256 MiB is reported as failed instead of being read into memory, and decoding keeps to the 512 MiB allocation limit of the image decoder. `game-media-vault derive-thumbnails --max-edge 256` renders the thumbnails every retained original lacks. A longest edge of 0 is refused as an invalid request. The desktop shell renders them through the `derive_thumbnails` command, on a blocking worker, and its `gmv-object` protocol serves Derived Assets by hash like originals. The desktop Library shows the 256-pixel thumbnail of an original when one has been rendered, and the original itself when it has none or the thumbnail cannot be shown. Its "Render thumbnails" action renders the 256-pixel thumbnails every retained original lacks, reports how many were rendered, how many originals could not be and how many were skipped as unsupported formats, and shows them (also after a failure that left some rendered), searching again with the filters of a search submitted meanwhile. A rendering stays attached to its vault: loading that vault again shows it still running, while another vault can render its own.

A transformation must never mutate the original object.

## 17. 3D Packaging

Game Media Vault should support both downloaded 3D assets and generated 3D packaging.

Generated packaging uses packaging-family templates with known geometry and expected texture slots. Examples include jewel cases, DVD cases, DS/3DS cases, cardboard boxes, clamshell cases, cartridges, optical discs, and floppy packaging.

The `Packaging` Coverage Profile determines whether enough accepted assets exist to build a usable generated model.

A generated packaging model is a Derived Asset of the release's front scan whose recipe names the exact back and spine scans it is textured with (`packaging-model-cardboard-box-<back hash>-<spine hash>`), so the same three scans give one model, built once and shared by every release scanned with them. Generation considers every Release Edition. One whose packaging family is unknown or has no template yet (only cardboard boxes have one so far) is counted as without a template. One whose Packaging Coverage Profile still misses Asset Types is listed with what it misses. The others are built from the Preferred Asset of each texture slot (front, back and spine), unless the model of those exact scans already exists. A better scan of a slot becomes its Preferred Asset and gives a new model, while the earlier one stays recorded. Every library release shows the model of its current Preferred Assets once it is built, as its `packaging_model`; models of earlier scans stay listed among the Derived Assets of their front scan. Scans are only read: a model that fails to build is reported with its release, and the others still build. `game-media-vault derive-packaging-models` builds the models every complete release lacks and prints how many it built, how many were already built, which releases miss which scans, how many have no template and which failed. The desktop shell builds them through the `derive_packaging_models` command, on a blocking worker. The desktop Library's "Build 3D boxes" action builds them and reports how many were built and already built, and how many releases miss scans, have no template or failed. It then shows them, even after a failure that left some built, and, like a thumbnail rendering, a build stays attached to its vault. A release with a model shows it in an interactive preview turned by dragging, whose 3D engine loads only once a model is shown; without WebGL, or when the model cannot load, the preview says it is unavailable and the Library goes on. A release without a model says which Packaging scans its 3D box still needs, that the box is not built yet, or that its packaging family has no 3D template yet. The webview may fetch vault objects, and the blob URLs the preview decodes textures from, but no other destination beyond the shell.

Models are glTF 2.0 binary files (`model/gltf-binary`, `.glb`) that embed their textures, so each stands alone, and the same scans always give the same bytes. The cardboard box template is one unit tall, as wide as the front scan and as deep as the spine scan for that height, with its front facing +Z, the spine on both sides and plain top and bottom edges. A spine scanned lying down (wider than tall) is turned clockwise to stand upright. Textures are scaled down to fit 2048 pixels and encoded as JPEG, or as PNG when the scan has transparency. Scans are decoded like thumbnail originals, as the format their media type records, and a scan larger than 256 MiB is refused; a scan that cannot be read fails its model, naming its slot.

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

Review Items show the competing Release Editions or metadata values, thumbnails/previews, evidence from each Source, matching score breakdown, and actions to accept, reject, merge, split, or defer. A preview downloads the candidate through the registered connector of its own Source.

### Library

Once a vault is open, the Library view imports a reference catalog file (a No-Intro or Redump datafile, or a MAME software list with the MAME release it came with) through the same use case as the CLI, read up to a bound and on a blocking worker; it reports the releases imported and the malformed records skipped, then shows the imported releases. A file that cannot be read or does not parse is reported; the Library is then shown again, since a failure may come after earlier batches of releases were recorded.

### Sources

Each Source displays:

- enabled/disabled state;
- available acquisition methods;
- authentication state when applicable;
- supported Asset Types;
- supported Platforms where known;
- current quotas/rate limits where discoverable;
- recent success/error statistics.

The desktop Sources view, which needs no vault, lists every registered Source with the Asset Types it acquires and its acquisition method, read from the same capabilities planning uses; `game-media-vault source list` prints the same descriptions as JSON. Executions record each Source failure in the vault as history: the run, the stage (discovery, which includes checking its part of the plan again, or download), what its connector reported, and when, in recording order. A later success leaves earlier failures as they are, and media a Source no longer serves (HTTP 404 or 410) completes as unavailable rather than failing. `game-media-vault source failures --latest 5` summarizes them per Source in source id order, with how many each Source had in all and its latest ones, newest first; Sources that never failed are left out. Connectors report no credentials in their errors, so recorded messages hold none. With a vault loaded, the desktop Sources view reads them again each time it is shown, through the `list_source_failures` command, and shows how many failures each Source recorded and its 3 latest (time in UTC, stage, run and message), or that it recorded none; another vault's failures never show. Enabled state, authentication, supported platforms, quotas and statistics are not shown yet.

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

Requests should also be serializable to a declarative file so large acquisitions can be reproduced on another machine. `game-media-vault run export <id>` prints the request of a run as a document `{ "format_version": 1, "request": … }` holding the request alone (no run state, Source credentials or vault paths), and `game-media-vault run start <file>` starts a run from such a document in any vault, planning it as `acquire` does. A document of another format version is `unsupported`; one that cannot be read, does not parse or holds a key the format does not know (a misspelled requirement would otherwise be dropped) is an invalid request. Unattended machines then run it with `game-media-vault run execute <id>`, which prints the run it leaves as JSON on stdout, even when the execution fails: the error goes to stderr and the exit code follows its kind, so a script can tell both where the run stands and why it stopped. Executing the run again, as after a restart, resumes its persisted work without discovering its Sources again.

Errors are classified into stable kinds shared by every frontend. The desktop shell returns them as `{ kind, message }`; the CLI prints the message on stderr and exits with a kind-specific code:

| Exit code | Kind | Meaning |
| --- | --- | --- |
| 0 | — | success |
| 1 | `external` | storage, network or another port failed |
| 2 | `invalid_request` | invalid arguments or request, such as accepting a Release Edition the Review Item does not offer (also command-line usage errors) |
| 3 | `not_found` | unknown run or Review Item, or a missing Release Edition an operation relies on |
| 4 | `conflict` | the current state forbids the operation |
| 5 | `unsupported` | valid but not supported for this Source or plan yet |
| 6 | `source_failure` | a Source broke the connector contract, e.g. malformed metadata or a reference catalog that does not parse |

The desktop webview never reads files directly. Original objects of the opened vault are served read-only by the `gmv-object` protocol, addressed by their BLAKE3 hash only (`400` for anything else, `404` for a missing object, `409` without an open vault), with a media type derived from the object's signature and `nosniff`; the Content Security Policy allows images from that protocol alone besides the app itself. Opening a vault returns its identity, the canonical form of its path (written losslessly, and an open whose path cannot be resolved fails rather than fall back to the path as typed): running executions, thumbnail renderings and every check that a result still belongs to the vault shown are keyed by it, so equivalent spellings of one path (relative or absolute, `.` and `..` components, or a different case on Windows) are one vault, while the vault path field keeps showing what the user typed.

## 22. Performance Principles

- Streaming downloads; avoid loading large media into memory unnecessarily.
- Hash while streaming when practical to avoid an extra full-file pass.
- Bounded channels/queues to provide backpressure.
- Batched database writes where durability semantics permit it.
- Explicit indexes for source IDs, release identifiers, hashes, review state, and library filtering.
- Lazy media decoding; inspect headers/metadata before full decode where possible.
- Concurrent independent source work under per-source limits.
- No UI dependency in background workers.
- Desktop commands that may wait on a Source (starting a run, whose plan check can consult it, executing a run, loading a review preview) run on blocking workers so the window stays responsive.
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

`game-media-vault verify`, and the desktop `verify_vault` command on a blocking worker, compare the catalog with the stored bytes and repair nothing. Every original a retained Asset references and every recorded Derived Asset is hashed again: one the store lacks is reported missing, one whose bytes hash to another address corrupt, and one that cannot be read (or whose recorded hash is not a lower-case BLAKE3 hash, which is never looked up) unreadable, without stopping the verification. An output several Derived Assets share is checked and reported once. Stored originals no retained Asset references (such as those below the quality requirements of their run) are reported unreferenced; outputs that only Derived Assets reading such an original record (a packaging model reads its front, back and spine scans), and derived files no record lists, orphaned; files left in `staging/` interrupted (except those of stores this process is still running). The store is listed before the catalog is read, so an object another task stores and records meanwhile is not reported unreferenced; staging files of another process still storing look interrupted. The report says whether the vault is healthy, that is whether it found nothing. `game-media-vault repair` then applies only the repairs it is given, each verified again afterwards: `--remove-interrupted-staging` deletes staging files, `--remove-orphaned-derived` forgets the Derived Assets reading an unreferenced original and deletes the derived files no remaining record lists (an output a referenced original shares stays), `--reset-damaged-derived` forgets missing and corrupt ones (deleting corrupt files) so they render again, and `--collect-unreferenced-originals` deletes unreferenced originals. Missing, corrupt and unreadable originals are never deleted or rewritten, since the vault cannot recover their bytes, and no unreadable file is deleted; the summary lists the Derived Assets forgotten and the files deleted; a repair without any action is an invalid request. Repairs assume no other process uses the vault, since a running store and an interrupted one look alike; only files named by a lower-case hash and stored at that hash's address are treated as objects. Work whose state contradicts its run or its Review Item, so that no execution will process it, is reported stale: work queued in a completed run (a run completes only once its queue is empty), and work parked on a Review Item a decision already closed (accepted, rejected, auto-resolved or superseded) in a run that was not cancelled, which that decision should have requeued or completed. Work queued in a running or paused run, parked on a pending or deferred Review Item (even in a completed run), or left in a cancelled run, which keeps the work its cancellation abandoned, is not. No repair settles stale work yet. Re-inspecting the media of originals recorded before media inspection is not covered yet.

## 24. Configuration and Secrets

Source credentials and API tokens are configuration, never catalog metadata. Secrets must use OS-appropriate protected storage when available and must not be written into exported Acquisition Requests, logs, or source provenance records.

## 24a. Releases

Pushing a `v*` tag, or dispatching the Release workflow with an existing tag, publishes a GitHub Release built from that tagged commit only. The tagged source is verified first (frontend tests and build, Rust formatting, lints and tests); then the desktop app (with its frontend embedded) and the CLI are built for Windows x86-64, Linux x86-64 and macOS on Apple silicon, each packaged as `game-media-vault-<tag>-<target>` (a zip on Windows, a tar.gz elsewhere) holding `game-media-vault-desktop` and `game-media-vault`. The release, with these archives and the source archives, is created only once every target has built: a failed verification or build publishes nothing.

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

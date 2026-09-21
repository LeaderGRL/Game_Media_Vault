# Game Media Vault acquisition and cataloging platform

## Problem Statement

Collectors, preservation workflows, and downstream games need media for specific video-game releases rather than a single generic image for a title. Existing data is fragmented across APIs, public websites, repositories, downloadable catalogs, and specialist archives. Different providers describe releases differently, expose different asset types, and often contain conflicting or incomplete metadata.

The user needs one desktop and command-line tool that can select exactly what to collect, acquire it from multiple chosen sources, preserve originals, distinguish regional and commercial Release Editions, expose uncertainty for review, and make library completeness visible.

## Solution

Build Game Media Vault as a Rust acquisition and cataloging engine with a thin Tauri desktop frontend and a first-class CLI. The engine will use pluggable Connectors for APIs, public web extraction, datasets, repositories, and local imports. Acquisition Requests will explicitly select Sources, Platforms, games, Asset Types, regions, languages, optional quality requirements, limits, and a Retention Policy.

Original Assets will be immutable and stored in a BLAKE3-addressed object store while SQLite stores the catalog, provenance, assertions, matching evidence, work state, and relationships. Derived Assets will be generated separately. Conflicting provider metadata will remain as Release Assertions; uncertain matches will become Review Items. Coverage Profiles will make partial and complete Release Editions visible according to packaging family and intended use.

## User Stories

1. As a collector, I want to select one or more Sources, so that I control where data is acquired from.
2. As a collector, I want source selection to be mandatory, so that a run never contacts providers I did not select.
3. As a collector, I want an explicit Auto source strategy, so that I can intentionally let the engine combine compatible providers.
4. As a collector, I want to select one or more Platforms, so that acquisition is limited to systems I care about.
5. As a collector, I want to target all games on selected Platforms, so that I can build broad archives.
6. As a collector, I want to target individual games, so that I can perform focused acquisitions.
7. As a collector, I want to cap the number of games in a run, so that I can test a source or workflow on a bounded sample.
8. As a collector, I want to filter by region, so that I can focus on specific markets.
9. As a collector, I want to filter by language, so that I can collect the variants useful to me.
10. As a collector, I want every Asset Type to be independently selectable, so that a request can download only box fronts when that is all I need.
11. As a collector, I want hierarchical Asset Type selection, so that I can select a whole family or precise media such as Box Front, Spine, Disc, Manual, Screenshot, or Video.
12. As a collector, I want optional minimum quality requirements, so that low-resolution media can be excluded when appropriate.
13. As a collector, I want a best-available quality mode, so that the engine can prefer the strongest candidate without a fixed resolution threshold.
14. As an archivist, I want original files preserved byte-for-byte, so that later processing cannot destroy source material.
15. As an archivist, I want crops, resizes, normalized images, extracted manual pages, textures, and generated 3D media stored separately, so that originals remain reproducible evidence.
16. As an archivist, I want identical files from different Sources deduplicated physically, so that storage is efficient without losing provenance.
17. As a collector, I want a Keep Everything policy, so that multiple valid scans of one Asset Type can coexist.
18. As a collector, I want a Keep Best Per Type policy, so that a compact library can retain the preferred candidate for each need.
19. As a collector, I want rejected candidate metadata retained, so that I can reconsider selection decisions later without rediscovering everything.
20. As a collector, I want each physical or commercial Release Edition represented separately, so that regional, serial, revision, packaging, language, and bundle differences are preserved.
21. As a researcher, I want provider-specific Release Assertions retained, so that conflicting metadata is not silently overwritten.
22. As a researcher, I want canonical values to retain provenance and confidence, so that I can understand why the library currently believes a value.
23. As a reviewer, I want uncertain matches routed to Review Items, so that questionable media cannot silently contaminate an edition.
24. As a reviewer, I want to see competing editions, previews, source evidence, and score explanations, so that I can resolve a Review Item efficiently.
25. As a reviewer, I want my decisions persisted, so that previously resolved cases do not repeatedly require manual work.
26. As a collector, I want to see Partial, Packaging Complete, Physical Complete, and Archival Complete states, so that missing media is obvious.
27. As a collector, I want completeness requirements to vary by packaging family, so that a jewel case and a cardboard box are evaluated correctly.
28. As a collector, I want to browse Game -> Release Edition -> Asset, so that closely related variants remain understandable.
29. As a collector, I want library filters for Platform, region, language, Source, Asset Type, completeness, and review state, so that large libraries remain usable.
30. As a collector, I want reusable Acquisition Request presets, so that common workflows such as Manuals Only or 3D Box Builder are quick to repeat.
31. As a collector, I want runs to be paused and resumed, so that long acquisitions fit around machine availability.
32. As a collector, I want interrupted runs to recover after restart, so that multi-hour or multi-day acquisitions do not lose completed work.
33. As a collector, I want per-Source rate limiting and retries, so that one provider's constraints do not destabilize the entire run.
34. As a collector, I want source capabilities visible in the UI, so that I know which providers can supply a requested Asset Type.
35. As a collector, I want current quotas and recent source errors visible when available, so that stalled acquisition is understandable.
36. As a power user, I want the same acquisition engine available from a CLI, so that I can run it unattended on a server or NAS.
37. As a power user, I want Acquisition Requests serializable, so that large runs are reproducible across machines.
38. As a maintainer, I want Connectors isolated behind stable capabilities, so that adding or changing one provider does not reshape the application core.
39. As a maintainer, I want source-specific labels preserved while mapping them to canonical Asset Types, so that new provider vocabulary does not require destructive normalization.
40. As a maintainer, I want every Asset to record provenance and source-policy metadata, so that future redistribution decisions can be evaluated per source and asset.
41. As a maintainer, I want public web Connectors to operate within normal public access boundaries, so that acquisition does not depend on bypassing authentication, CAPTCHA, paywalls, or access controls.
42. As a maintainer, I want a vault verification command, so that missing, corrupt, orphaned, or interrupted files can be detected.
43. As a maintainer, I want structured run identifiers and failure reasons, so that acquisition failures are diagnosable without exposing secrets.
44. As a developer, I want the core independent from Tauri, so that desktop and unattended CLI workflows share exactly the same behavior.
45. As a developer, I want behavior implemented through red -> green TDD slices, so that tests describe public behavior before production code exists.
46. As a developer, I want each coherent modification or feature committed separately after its checks pass, so that history remains reviewable and regressions are easy to bisect.
47. As a maintainer, I want GitHub CI to run formatting, linting, tests, and builds as the project grows, so that broken changes cannot silently enter the main branch.
48. As a maintainer, I want tagged releases to produce distributable artifacts when the Rust/Tauri targets exist, so that delivery is reproducible from GitHub.

## Implementation Decisions

- The product is a Rust workspace with separate domain, application, persistence, storage, acquisition, connector, media-processing, CLI, and Tauri adapter responsibilities.
- Tauri remains a thin frontend adapter. Core domain and application behavior has no Tauri dependency.
- SQLite is the initial metadata catalog for the single-user application, behind repository interfaces that keep persistence details out of the domain.
- Original binary media is stored outside SQLite in an immutable content-addressed object store keyed by BLAKE3.
- Derived Assets always reference an original Asset and transformation recipe; they never overwrite originals.
- A Release Edition is the collectible identity boundary. Platform, territory, serial/product code, commercial edition, packaging, revision, bundle, meaningful printed-language variants, and identifiable reissues can distinguish editions.
- Providers contribute Release Assertions. Canonical values are derived from assertions with provenance and confidence rather than treating one provider as absolute truth.
- Acquisition Requests require at least one Source and at least one Asset Type. Regions, languages, and quality constraints are optional filters.
- Asset Types are hierarchical but individually selectable. Source-specific labels map to extensible canonical Asset Types while retaining the original label.
- Retention supports Keep Everything and Keep Best Per Type. Exact byte duplicates reuse storage while preserving all source relationships.
- Matching uses independent evidence such as source IDs, normalized titles, platform, serial codes, region, language, publisher, dates, packaging descriptors, and legitimate hash catalogs.
- Confidence thresholds are configurable. Medium-confidence matches create Review Items; low-confidence matches remain unattached.
- Coverage is defined by configurable Coverage Profiles, with initial Partial, Packaging Complete, Physical Complete, and Archival Complete states.
- Connector capabilities are declared explicitly. A Source can expose multiple acquisition methods such as API plus dataset or public web pages.
- Acquisition work is persisted, idempotent, bounded by global and per-Source concurrency, and resumable after process restart.
- Downloads are streamed and hashed while streaming when practical. Media decoding is lazy where possible.
- The CLI is a first-class frontend over the same application layer and supports reproducible serialized Acquisition Requests.
- GitHub Actions is the automation platform for CI and release delivery. Rust checks become required once the workspace exists; release automation produces artifacts from version tags once buildable targets exist.

## Testing Decisions

- Tests verify public behavior, not private implementation details. Refactors that preserve behavior should preserve tests.
- The primary test seam is the application/use-case boundary. Connectors, catalog repositories, object storage, clocks, and external network interactions are ports replaced by focused test doubles at that seam.
- Domain invariants that are naturally pure, such as Release Edition identity, Acquisition Request validation, Retention Policy decisions, and Coverage Profile evaluation, can be tested directly through their public domain APIs.
- Connector protocol parsing receives focused contract/fixture tests where behavior cannot be exercised economically through the application seam.
- Every implementation slice follows red -> green: one failing behavior test is observed before the minimum production change that makes it pass.
- Tests are added vertically with their implementation rather than writing a large speculative test suite up front.
- Persistence and object-store adapters receive integration tests for behaviors that depend on SQLite transactions, filesystem durability, hashing, or restart recovery.
- Network-dependent live-provider tests are not part of the default deterministic CI suite. Provider fixtures and contract tests cover parsing; optional live checks can be isolated later.
- The repository currently has no prior test suite, so the first Milestone 1 vertical slice establishes the testing conventions used by later modules.
- CI will enforce formatting, linting, tests, and build checks as those targets become present in the repository.

## Out of Scope

- Redistributing downloaded third-party media as a public asset service.
- Bypassing authentication, CAPTCHA, paywalls, access controls, or anti-bot challenges.
- Downloading game ROMs, disc images, executables, or other playable game content as part of the media-vault feature set.
- Treating downloaded media as automatically licensed for commercial redistribution.
- Multi-user hosted service architecture in the initial product.
- Final commercial licensing clearance for the future game that may consume this library.
- OCR and full-text manual search in the initial usable version; the architecture may add these later.
- Exhaustive 3D geometry generation for every historical packaging family in the first vertical slice.

## Further Notes

- Initial candidate media providers include ScreenScraper, GameTDB, EmuMovies, LaunchBox Games Database, MobyGames, TheGamesDB, Libretro Thumbnails, GamesDatabase, Gaming Alexandria, Sega Retro, ReplacementDocs, RetroMags, PSX DataCenter-style specialist references, VGMaps, RAWG, and SteamGridDB.
- Initial reference catalogs include No-Intro DATs, Redump DATs, and MAME Software Lists.
- Source support is deliberately open-ended; the registry is a seed list, not an allowlist.
- The first implementation milestone should be a narrow end-to-end vertical slice: domain model, persistent request/run, SQLite catalog, BLAKE3 object store, one simple provider Connector, one reference-catalog importer, a minimal CLI, and a minimal Tauri Acquire/Run/Library flow.

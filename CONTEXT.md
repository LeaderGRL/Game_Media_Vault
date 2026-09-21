# Game Media Vault

Game Media Vault catalogs collectible video-game releases and the media assets that describe them. The domain distinguishes the underlying game from each physical or digital release edition so regional, packaging, language, revision, and bundle differences are never collapsed accidentally.

## Catalog

**Game**:
The underlying video game work independent of platform, territory, packaging, or commercial edition.
_Avoid_: Release, edition, ROM

**Platform**:
A hardware or software ecosystem on which a game is released, such as PlayStation, Game Boy, Windows, or an arcade system.
_Avoid_: Console when the system is not necessarily a console

**Release Edition**:
A distinct collectible release of a game identified by meaningful commercial differences such as platform, territory, product code, packaging, revision, bundle, or edition.
_Avoid_: Version, copy, ROM

**Release Assertion**:
A source-specific claim about a Release Edition, such as its region, languages, product code, title, publisher, or release date. Conflicting assertions are retained rather than overwritten.
_Avoid_: Truth, metadata row

**Canonical Value**:
The value currently selected by Game Media Vault from one or more Release Assertions, with provenance and confidence preserved.
_Avoid_: Ground truth

## Media

**Asset**:
An original media file associated with a Release Edition, Game, Platform, or related collectible object. Every Asset retains its provenance and original bytes.
_Avoid_: Image when the media may be PDF, video, audio, archive, or 3D data

**Asset Type**:
The precise role of an Asset, such as box front, box back, spine, cartridge, disc, manual, screenshot, video, logo, flyer, advertisement, map, PCB, arcade cabinet, 3D model, or texture.
_Avoid_: Category when a precise media role is meant

**Derived Asset**:
A generated file produced from an original Asset, such as a crop, resize, normalized image, extracted page, texture, or generated 3D representation. It never replaces its original Asset.
_Avoid_: Original, source asset

**Asset Candidate**:
A discovered remote or imported media item that has not yet necessarily been accepted into the library.
_Avoid_: Asset until it is persisted as part of the library

**Preferred Asset**:
The currently preferred Asset for a Release Edition and Asset Type. Other matching Assets may still be retained.
_Avoid_: Best asset when the preference is policy-dependent

## Acquisition

**Source**:
A provider from which catalog data or media can be obtained, including APIs, public web pages, downloadable datasets, repositories, or local imports.
_Avoid_: Website when the provider may not be a website

**Connector**:
The source-specific integration that discovers catalog records and Asset Candidates through a supported acquisition method.
_Avoid_: Scraper when the integration may use an API, dataset, or local import

**Acquisition Request**:
The user-defined selection of platforms, games, regions, languages, asset types, sources, quality constraints, limits, and retention policy for one collection run.
_Avoid_: Crawl when the request may target explicit games only

**Acquisition Run**:
One persisted execution of an Acquisition Request, including its progress, discoveries, downloads, failures, retries, and final statistics.
_Avoid_: Session

**Retention Policy**:
The policy controlling whether all accepted Assets are retained or only the preferred Asset per matching scope.
_Avoid_: Deduplication policy

## Review and Coverage

**Review Item**:
An uncertain match or conflicting decision that requires human confirmation before it can affect canonical library data.
_Avoid_: Error when uncertainty is expected

**Coverage Profile**:
The set of Asset Types required to consider a Release Edition complete for a particular purpose, such as 3D packaging, physical archival, or full archival.
_Avoid_: Completeness when referring to the rule rather than its result

**Coverage Status**:
The evaluated completeness of a Release Edition against a Coverage Profile, such as Partial, Packaging Complete, Physical Complete, or Archival Complete.
_Avoid_: Download status


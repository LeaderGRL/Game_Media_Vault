# Sources

What each implemented Source provides, where it is reached, and what it needs to be configured.
No implemented Source needs an account, an API key or any other secret. Sources that will need
one keep it in protected storage, never in the catalog, exported requests, logs or provenance
(SPEC §24).

Every HTTP request identifies itself as `game-media-vault/0.1`. It retries transient failures
(connection failures, timeouts, HTTP 429 and 5xx) up to four attempts in all, with a jittered
backoff that honours `Retry-After`, and resumes a download its connection cut short when the
Source proves the media unchanged (SPEC §12). Executions record each failure of a Source in the
vault; `game-media-vault source failures` and the desktop Sources view summarize them.

## Media Sources

These Sources are registered connectors; `game-media-vault source list` describes them, and
`game-media-vault source disable <source>` keeps one out of every acquisition on this machine
until `source enable <source>`. A Source that needs an API key reads it from this machine's
secure credential store, where `game-media-vault source key set <source>` stores the key it reads
from standard input (ADR 0005).

### Libretro Thumbnails (`libretro-thumbnails`)

- **Provides:** Box Front, Screenshot and Title Screen, from the `Named_Boxarts`, `Named_Snaps`
  and `Named_Titles` folders of the `libretro-thumbnails` repositories.
- **Reached at:** `raw.githubusercontent.com`. The repository list is read from the
  `.gitmodules` file of `libretro-thumbnails/libretro-thumbnails`, and images from each
  platform's repository.
- **Configuration:** none. A platform is served when a repository bears its name, such as
  `Nintendo - Nintendo Entertainment System`.
- **Limits:** it holds one image per game and type, named after the release, so it refuses
  region and language filters and selections of every game of a platform.

### LaunchBox Games Database (`launchbox-games-db`)

- **Provides:**
  - box fronts, backs, spines and 3D renders;
  - cartridge fronts and backs, and discs;
  - gameplay screenshots, title screens, clear logos and background fan art;
  - front flyers;
  - arcade marquees, cabinets, control panels and circuit boards.

  The image type each comes from is kept as its source label.
- **Reached at:** `gamesdb.launchbox-app.com/Metadata.zip`, the daily dataset, and
  `images.launchbox-app.com` for the images. The dataset is kept once for the whole machine, in
  the OS cache directory under `game-media-vault/launchbox`, and downloaded again only once
  LaunchBox republishes it.
- **Configuration:** none. Platforms are mapped to LaunchBox's names, for every platform whose
  packaging family the coverage table knows except the Nintendo DSi, which LaunchBox does not
  list; MAME and FBNeo releases are looked up under its `Arcade` platform.
- **Limits:** it refuses language filters and platforms it has no name for. The dataset is
  large, so the first discovery on a machine, and the first after LaunchBox republishes it,
  download it whole.

### SteamGridDB (`steamgriddb`)

- **Provides:** community logos (Logo), icons (Icon) and heroes, its wide background artwork
  (Wallpaper / Artwork), best voted first. The collection and style each comes from, such as
  `logos: official`, is kept as its source label. Grids, its library capsules, have no Asset
  Type yet and are not acquired.
- **Reached at:** the API at `www.steamgriddb.com/api/v2`, which finds each requested game by
  name and lists its media, and `cdn2.steamgriddb.com` for the images, downloaded without the key.
- **Configuration:** an API key from the user's own SteamGridDB account, kept in this machine's
  credential store (ADR 0005). To get one:
  1. Sign in at `https://www.steamgriddb.com` (it signs in through a Steam account).
  2. Open the API page of the preferences, `https://www.steamgriddb.com/profile/preferences/api`,
     and generate a key.
  3. In the desktop Sources view, paste the key in SteamGridDB's API key field and choose
     **Store key**. Or run `game-media-vault source key set steamgriddb`, paste the key and press
     Enter: the key is read from standard input, so the shell never keeps it in its history.
- **Limits:** without a stored key it is left out of every plan, with the reason. It needs an
  explicit game selection, since it lists no platform's games, and refuses region and language
  filters. A game is acquired only when SteamGridDB names it exactly as the request does,
  regardless of case and punctuation; its media apply to every platform the request names.

## Reference catalogs

These read files the user downloads and passes to an `import-*` command; they contact no
network.

### No-Intro (`no-intro`) and Redump (`redump`)

- **Provides:** Release Editions with title, region, revision, dump names and checksums, from
  Logiqx XML datafiles: `import-no-intro` and `import-redump`.
- **Configuration:** the datafile path and the number of releases to read.

### MAME software lists (`mame-software-lists`)

- **Provides:** Release Editions with the metadata and dumps of each software, from a MAME
  `hash/<system>.xml` list: `import-mame-software-list`.
- **Configuration:** the list path, the number of releases to read, and optionally the MAME
  release the list came with (`--mame-version`), which the list does not record itself.

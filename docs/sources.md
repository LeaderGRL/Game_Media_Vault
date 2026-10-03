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

### TheGamesDB (`thegamesdb`)

- **Provides:** box fronts and backs (Box Front, Box Back), screenshots, title screens, clear
  logos (Logo) and fan art (Wallpaper / Artwork). The image type and box side each comes from,
  such as `boxart: front`, is kept as its source label. Banners have no Asset Type and are not
  acquired.
- **Reached at:** the API at `api.thegamesdb.net`, which lists its platforms, finds each requested
  game by name on them and lists the media of the games found in one paged request, and
  `cdn.thegamesdb.net` for the images, downloaded without the key.
- **Configuration:** an API key from the user's own TheGamesDB account, kept in this machine's
  credential store (ADR 0005). The API takes the key as a query parameter, which the connector
  adds only to the requests it sends: errors name the request without it, and the request
  follows no redirect, which would carry it elsewhere. To get one:
  1. Create an account and sign in at `https://thegamesdb.net`.
  2. Follow **API Access Request** at the bottom of the page, fill in the request and confirm
     it. The key appears in the list on that page once TheGamesDB approves it.
  3. In the desktop Sources view, paste the key in TheGamesDB's API key field and choose
     **Store key**. Or run `game-media-vault source key set thegamesdb`, paste the key and press
     Enter.
- **Limits:** a key has a monthly allowance of requests. A discovery takes one request for the
  platform list, up to five pages of search per requested game and platform, and at most ten
  pages of images for each batch of twenty games found. Without a stored key it is left out of
  every plan, with the reason. It needs an explicit game selection and refuses region and
  language filters. A platform is served when TheGamesDB names it with the same words,
  regardless of case and punctuation, with or without the words in parentheses it may add, such
  as `Nintendo Entertainment System (NES)` for
  `Nintendo - Nintendo Entertainment System`. A qualifier of the requested name, such as
  `(Digital)`, counts, so a digital platform is never taken for the physical one. A few catalog
  names TheGamesDB words otherwise are known by alias, such as `Super Nintendo (SNES)` for
  `Nintendo - Super Nintendo Entertainment System`, and `Sega Genesis` with `Sega Mega Drive` for
  `Sega - Mega Drive - Genesis`. A game is acquired only when TheGamesDB names it exactly as the
  request does, regardless of case, punctuation and spacing, on one of those platforms.


### PSX DataCenter (`psx-datacenter`)

- **Provides:** high-resolution scans of PlayStation box fronts and backs (Box Front, Box Back)
  and screenshots. The side, and an edition other than the standard one, `GH` for Greatest Hits
  or `P` for Platinum, is kept as the source label (`front: GH`), and the edition as the
  candidate's; a scan of an edition the site names otherwise is left out rather than taken for
  the standard one. Inlays and advertisements have no Asset Type yet and are not acquired. Every
  candidate is recorded on `Sony - PlayStation`, and with the region of the list it came from:
  scans and screenshots another region's directory holds are left out.
- **Reached at:** the public website `psxdatacenter.com`: the list of each region it is asked
  for (`ulist.html` for NTSC-U, `plist.html` for PAL, `jlist.html` for NTSC-J), then the page of
  each requested game found in it, then the images, all on that site only. It is read as a
  well-behaved client: only as its robots.txt allows `game-media-vault`, read once per site, and
  at least a second, or the site's longer `Crawl-delay`, between two requests, one pace for the
  whole process. Each request is sent once, following no redirect: a failure defers the work to
  a later execution instead of asking again at once. A site asking for more than 30 seconds
  between requests is left alone.
- **Configuration:** none.
- **Limits:** it covers only `Sony - PlayStation` (also named `PlayStation`, `PSX` or `PS1`), and
  refuses a request naming any other platform, which it would leave unserved. It needs an
  explicit game selection, since reading every game's page would take hours at that pace.
  Regions narrow the lists read: `USA` and `North America` read NTSC-U; `Europe` and `PAL` read
  PAL, whose releases are recorded in Europe; `Japan` reads NTSC-J. Any other region, and
  language filters, are refused. A game is acquired only when exactly one row of a list names
  it as the request does, regardless of case, punctuation, spacing and the number of discs the
  list adds: several rows sharing a title are releases its title alone cannot tell apart. A
  list without any game, as during an outage, fails the discovery as invalid source data.

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

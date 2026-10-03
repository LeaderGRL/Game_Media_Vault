# Sources

What each implemented Source provides, where it is reached, and what it needs to be configured.
Some Sources need an API key, or the identifiers of an account, that the user gets from the
Source itself. Each is kept in protected storage, never in the catalog, exported requests, logs
or provenance (SPEC §24).

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
from standard input (ADR 0005). A Source that asks for several credentials takes `--field <name>`
to say which one is given, and `source key clear <source>` forgets them all, or with `--field`
only the one named; an API key is stored under the Source's id, any other credential under
`<source>/<field>`.

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
  `logos: official`, is kept as its source label. Grids, its library capsules, are not
  acquired: they are fan-made launcher covers rather than scans of packaging, and the maintainer
  chose to leave them out (#158).
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

### ScreenScraper (`screenscraper`)

- **Provides:** box fronts, backs and spines (Box Front, Box Back, Spine), 3D boxes (Box 3D
  Render), scans of the cartridge or disc (Cartridge Front or Disc, by what the platform's games
  come on), manuals as PDF (Manual), screenshots, title screens, gameplay videos as MP4, both
  the original `video` and the `video-normalized` encoding (Gameplay Video), wheels (Logo), fan
  art (Wallpaper / Artwork) and arcade flyers (Flyer). The media type and region code, such as
  `box-2D (us)`, is kept as its source label, and the region it stands for (`us` as USA, `eu` as
  Europe, `jp` as Japan, `wor` as World, a country by its own) as the candidate's; a media of no
  region, or of one ScreenScraper alone names, is recorded in no region. Mixes and other composites
  have no Asset Type and are not acquired.
- **Reached at:** the API at `api.screenscraper.fr`, which searches each requested game by name
  on the ScreenScraper system of its platform, describes a game found without its media, and
  serves the media themselves. Every request goes to that API, whatever server a media's own
  address names, and a media ScreenScraper does not serve itself is left out.
- **Configuration:** ScreenScraper asks every software for developer credentials, and lets each
  user add their own account, which brings their own quota and keeps requests going when it
  closes its API to anonymous use. All of them are kept in this machine's credential store (ADR
  0005) and sent only as query parameters of the requests the connector sends: errors name the
  request without them, requests follow no redirect, and the addresses ScreenScraper gives its
  media, which carry them, are never kept: each candidate is located by a path naming the media
  instead, such as `https://api.screenscraper.fr/api2/mediaJeu.php/57/1234/box-2D(us)`. To get
  them:
  1. Create a free account at `https://www.screenscraper.fr` and sign in. Its user name and
     password are the optional account credentials.
  2. Ask ScreenScraper for developer credentials on its forum, in the section for developers of
     scraping software: say that they are for your own use of Game Media Vault, a free and
     open-source media archiver (`https://github.com/LeaderGRL/Game_Media_Vault`), which
     identifies itself as `game-media-vault`. ScreenScraper answers with a developer id and
     password. Its API may be used only by entirely free software, which Game Media Vault is.
  3. In the desktop Sources view, ScreenScraper asks for its Developer id, Developer password,
     Account user name and Account password: paste each in its field and choose **Store key**.
     Or run `game-media-vault source key set screenscraper --field dev-id`, then with
     `--field dev-password`, `--field user-id` and `--field user-password`, pasting each and
     pressing Enter. The account is sent only once both its user name and password are stored.
- **Limits:** an account may send one request at a time, and a daily quota of requests, which
  free accounts keep low; the connector sends one request at a time, whatever runs send them,
  downloads included, and holds each media whole before the next request leaves. A discovery
  takes one search per requested game and ScreenScraper system, which serves each of its
  platforms, one more per game found without its media, and each media
  downloaded one more. ScreenScraper refusing a request for too many at once (HTTP 429) is asked
  again later; a quota spent for the day (HTTP 430) fails until the next day. Without developer
  credentials it is left out of every plan, with the reason. It needs an explicit game selection,
  takes region filters it has a code for and refuses language filters. It serves the platforms
  it knows a ScreenScraper system for, by their catalog names: the Nintendo, Sega, Sony, NEC,
  SNK, Bandai and Atari consoles and handhelds, ColecoVision, Intellivision, Vectrex, 3DO, CD-i,
  Xbox, Xbox 360 and Xbox One, and `MAME` and `FBNeo - Arcade Games` as ScreenScraper's arcade system; a
  request naming another is refused. A game is acquired only when one of its ScreenScraper names
  is the requested title, regardless of case, punctuation and spacing.

### RAWG (`rawg`)

- **Provides:** screenshots and each game's background image (Wallpaper / Artwork), kept with
  the source label `screenshot` or `background`. RAWG's images record no region and apply to
  every requested platform RAWG lists the game on.
- **Reached at:** the API at `api.rawg.io`, which searches each requested title once, whatever
  the platforms, and lists with each game it finds the platforms it is on, its background image
  and screenshots; and `media.rawg.io` for the images, downloaded without the key and following no redirect. An image
  another server serves is left out.
- **Configuration:** an API key from the user's own RAWG account, kept in this machine's
  credential store (ADR 0005). The API takes the key as a query parameter, which the connector
  adds only to its searches: errors name the request without it, and the request follows no
  redirect. RAWG's free plan is meant for personal and non-commercial projects, and asks for an
  active link to RAWG wherever its data shows: the desktop Sources view, the provenance of
  its media in the Library and the evidence of its candidates in Review link to
  `https://rawg.io`, opened in the system browser. To get one:
  1. Create a free account and sign in at `https://rawg.io`.
  2. Open `https://rawg.io/apidocs` and choose **Get API Key**. Fill in the form, naming Game
     Media Vault and its repository (`https://github.com/LeaderGRL/Game_Media_Vault`) as the
     project, and accept the terms. The key shows on that page once RAWG issues it.
  3. In the desktop Sources view, paste the key in RAWG's API key field and choose **Store
     key**. Or run `game-media-vault source key set rawg`, paste the key and press Enter.
- **Limits:** a free key has a monthly allowance of requests. A discovery takes one search per
  requested title. Without a stored key it is left out of every plan, with the reason. It needs
  an explicit game selection and refuses region and language filters. A platform is served when
  RAWG names it with the same words, regardless of case and punctuation, with or without the
  maker the catalog names first, such as `PlayStation 4` for `Sony - PlayStation 4` and
  `SEGA Saturn` for `Sega - Saturn`. A few catalog names RAWG words otherwise are known by
  alias, such as `SNES` for `Nintendo - Super Nintendo Entertainment System` and `Genesis` for
  `Sega - Mega Drive - Genesis`. A game is acquired only when RAWG names it exactly as the
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

### VGMaps (`vgmaps`)

- **Provides:** maps of games' levels and worlds (Map), ripped or drawn by the site's
  community, as images or, for some, PDF documents, each kept with the area and name its atlas gives it as the source label, such as
  `World 1 · 1-1`. Maps record no region and apply to the requested platform.
- **Reached at:** the public website `www.vgmaps.com`: the atlas page of each requested
  platform, such as `Atlas/NES/index.htm`, read once for every game requested on it, then the
  map images that game's table links, all on that site only. It is read as PSX DataCenter is:
  only as its robots.txt allows `game-media-vault`, at least a second, or the site's longer
  `Crawl-delay`, between two requests, each request sent once and following no redirect. Its
  pages are written in Windows-1252, which the connector decodes.
- **Configuration:** none.
- **Limits:** an atlas page holds every game of its platform, the largest over 5 MB, and a game
  may have dozens of maps, each downloaded at the site's pace. It needs an explicit game
  selection and refuses region and language filters. It serves the platforms it knows an atlas
  for: the Nintendo, Sega, Sony, NEC, SNK, Bandai and Atari consoles and handhelds,
  ColecoVision, Intellivision, CD-i, Xbox and Xbox 360, and `MAME` and `FBNeo - Arcade Games`
  as its arcade atlas; a request naming another is refused. A game is acquired when the title
  of its table names it exactly, regardless of case, punctuation, an article filed last, such
  as `Legend of Zelda, The` for `The Legend Of Zelda`, and Roman numerals, such as `Mega Man 2`
  for `Mega Man II`. In an atlas several platforms share, such as Game Boy and Game Boy Color, a
  table whose title adds one of them in parentheses, as `Prince Of Persia (Game Boy Color)`, serves
  that platform alone, and an unqualified one serves a platform without a table of its own.

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

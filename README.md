# Faustus

Faustus records and inspects Path of Exile economy data. The Rust workspace contains poe.ninja economy collection, historical dump imports, SQLite storage, and a market service. The Aurelia v2/TypeScript frontend runs in a browser or a Tauri v2 desktop shell. The previous GPUI implementation and legacy GGG integration are preserved in `crates/market-ui-deprecated` and the Rust crates, but normal service operation no longer collects GGG exchange history.

## Run Locally

Prerequisites: Rust 1.92+, Node.js 22.12+ (or a newer supported Node release), npm, and the [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/) (MSVC build tools and WebView2). Aurelia v2 currently uses the pinned `2.0.0-rc.2` release.

Install frontend dependencies once:

```powershell
Push-Location crates\market-ui
npm ci
Pop-Location
```

Start the backend and Windows Tauri app together:

```powershell
.\scripts\dev.ps1
```

The development script builds and starts the loopback backend, waits for its health endpoint, then launches Tauri/Vite in the foreground. Closing the desktop app stops the development backend. Service logs are written under `$env:TEMP\Faustus`. Use `.\scripts\dev.ps1 -Web` to start the same service with the browser frontend at `http://127.0.0.1:5173`, or `-Deprecated` to launch GPUI instead.

The client loads the poe.ninja economy leagues, uses the service's valid selection or defaults to the first entry (the current temporary softcore trade league), and tells the service which league to watch. The service validates that selection and starts collecting immediately. Changing the dropdown switches the collector to the newly selected league. League options are cached by the service for 15 minutes.

To run the backend separately:

```powershell
cargo run -p market-service -- serve
```

To trigger a single collection manually instead:

```powershell
cargo run -p market-service -- ingest-once --league "LEAGUE_ID"
```

Each `ingest-once` run collects supported poe.ninja economy categories, using the same provider cache and validators as the scheduled collector. It does not collect GGG exchange pages.

When running the backend separately, launch the desktop frontend in another terminal:

```powershell
cd crates\market-ui
npm run tauri -- dev
```

Use `npm run dev` instead for the browser frontend. Vite proxies `/api` and `/health` to `http://127.0.0.1:8787`; both clients use ordinary HTTP `fetch`, not Tauri commands or an HTTP plugin. The packaged desktop app also requires an independently running market service; it does not start, bundle, or stop one.

The **Current prices** tab is an economy workspace with grouped General, Equipment & Gems, Atlas, and Crafting navigation. It covers the supported PoE 1 economy categories, with Exchange/Stash source switching where the public API supports both. Search is category-local and independent of Gem flipping. Tables support sorting, 100-row batches, native/Chaos/Divine quotes, variant details, recent trend charts, and category-specific filters for item type, gem level/quality, links, corruption, map tier, and listing counts where supplied. Empty, failed, and unsupported source combinations are explicit. Prices are aggregate estimates, not executable buy/sell orders.

Click an item to see its available properties/modifiers and local price history. Charts retain native units, separate daily dumps from timestamped observations, and break at missing intervals rather than inventing prices. A history data table provides the same observations without relying on the chart. Range filters are relative to that item's newest saved observation, so they also work for old leagues. Historical quotes use same-day currency rates when available, never today's rates; unavailable conversions display a dash. The **Dataset** and **As of** selectors browse saved prices without switching the live collector or the Gem flipping league. A league can have both live feeds and imported history.

Historical table trends use the eight daily samples spanning the seven days ending on the selected date, never later prices. Missing days remain gaps; a seven-day percentage requires valid, nonzero baseline and end observations in the same currency. Icons are display-only enrichment from saved poe.ninja supported overview metadata (fetched from the Standard overview when no metadata is cached), served by Grinding Gear Games' `web.poecdn.com` image CDN. Metadata prefers Standard and Stash per item name, but retains Exchange-only items. The dump's broad Currency group also checks Fragment and other exchange-category metadata. Missing currency/fragment icons additionally check the full Standard Currency and Fragment `currencyDetails` catalogs through the shared overview cache, including entries without live prices (such as Unrelenting Timeless emblems). These icon-only entries never become price observations. Exact names are matched using compatible saved provider IDs or normalized variants (including map generations and influence variants); unambiguous artwork is a fallback. Missing currency/fragment icons without variants (including historical reliquary keys) also use exact, unambiguous names from [GGG's official trade static metadata](https://www.pathofexile.com/api/trade/data/static), cached in memory for 24 hours. Both sources accept only HTTPS images from `web.poecdn.com`. Missing or genuinely ambiguous matches stay blank; metadata-fetch errors are shown in the workspace. This never joins price identities, substitutes current prices, or supplies historical listing counts.

In item details, drag anywhere on the compact overview graph to select a custom period. The drag endpoints snap to saved observations, are sorted regardless of drag direction, and clamp to the full history's left/right endpoints even when released outside the graph. The start/end handles remain individually draggable, with arrow/Home/End keys available when focused. Double-click either handle (or press Enter while focused) to reset to all available history; presets also adjust the range without refetching. Period labels show dates only, while snapshot hover labels retain hours/minutes (UTC) without subsecond precision. The price chart has observation dots, adaptive price-axis increments, day labels, and subtle gridlines. Hover it to show a vertical guide and the nearest saved observation's date/time, price, currency, and granularity; missing prices are not interpolated. A thin volume chart below it shows provider-reported quote-currency volume at each saved snapshot, not summed daily trades. Volume is retained for newly collected observations; historical dumps and older snapshots have no recorded volume and display an explicit unavailable state instead of fabricated zeros. Price axes, volume, and the history table all follow the selected period. History rows scroll with the item detail page rather than inside a compressed nested pane.

Custom graph periods retain their selected dates while browsing other items, categories, sources, live leagues, or archived datasets in the current workspace session. Each history snaps those dates to its nearest available observations, clamping to an endpoint if necessary, without overwriting the original selection. Returning to an item with full coverage restores the original period, even after empty or failed history requests. Changing a handle or dragging a new range replaces the selection; reset clears it. Trailing-day presets retain their selected duration and remain relative to each item's newest observation.

Economy numbers use spaces as thousands separators and a decimal comma (for example, `1 234 567,89`), including table values, item details, chart axes/tooltips, volume, and observation counts. Stored and API numeric values retain their original machine-readable format.

Item details replace the category title with a `Category > Item` breadcrumb. Click the category in that breadcrumb to return to the list with its filters and selected graph period preserved.

The GGG history tab and active frontend market queries have been removed. Existing GGG database records, legacy read endpoints, and deprecated app code are retained.

### Historical economy dumps

Open **Historical data** in Current prices to discover published [poe.ninja economy dumps](https://poe.ninja/poe1/data), import or retry individual archives, see import counts/date coverage/errors, and browse completed leagues. **Discover dumps** checks for newly published archives, including Allflame once it appears; unpublished archives are not guessed or downloaded automatically.

Imports include only the named softcore challenge league. Hardcore, Ruthless, and permanent-league CSVs in the same ZIP are not imported. The initial selection is **Ancestors, Mirage, Phrecia 2.0, Keepers, and Mercenaries**. Imports are explicit, bounded, and idempotent. Incomplete jobs are hidden from historical queries and can be retried after failure/restart; a retry of the same checksum reuses existing observations without duplication. The importer streams semicolon-separated CSVs and preserves item IDs, base types, variants and link groups rather than joining prices by name. Archive IDs remain separate from live provider IDs when a reliable variant match is unavailable.

Per-archive OS locks beside the economy database prevent overlapping CLI/service imports. Locks are released automatically when a process exits; service restart recovery does not mark a still-running CLI job as failed.

CLI equivalents:

```powershell
cargo run -p market-service -- import --name "Ancestors"
cargo run -p market-service -- import --name "Ancestors" --file "C:\data\Ancestors.zip"
# Explicitly replace a completed archive when reprocessing it is needed:
cargo run -p market-service -- import --name "Ancestors" --replace
```

Use one database for live economy updates, historical dumps, and saved Gem flipping snapshots. `--database` selects it for every command; `FAUSTUS_DATABASE` provides a shared environment default. Without either, the default is `faustus.sqlite3` in the working directory.

```powershell
cargo run -p market-service -- --database "D:\Faustus\faustus.sqlite3" serve
cargo run -p market-service -- --database "D:\Faustus\faustus.sqlite3" import --name "Mirage"
.\scripts\dev.ps1 -Web -Database "D:\Faustus\faustus.sqlite3"
# Optional persistent default for new terminal sessions:
[Environment]::SetEnvironmentVariable("FAUSTUS_DATABASE", "D:\Faustus\faustus.sqlite3", "User")
```

The development launcher reads `FAUSTUS_DATABASE` from the process environment or the Windows user setting, so an already-open editor can use the persisted default too. Explicit `-Database` / `--database` arguments override that default. CLI commands launched from an existing terminal need `$env:FAUSTUS_DATABASE` set in that terminal or a new terminal session.

The optional `--economy-database` / launcher `-EconomyDatabase` (`FAUSTUS_ECONOMY_DATABASE`) override remains available for deliberately split deployments, but is unnecessary for the single-database setup. A separate economy store uses SQLite WAL with NORMAL synchronous mode and bounded per-connection caches for bulk imports: it remains crash-consistent, but an OS/power failure can lose the newest uncheckpointed observations, which can be recollected or reimported. A unified store uses the main database's unchanged durability settings. Allow space for the database, WAL, archive downloads, SQLite temporary files, and build artifacts. Parent directories for a custom database must already exist.

An upgrade of an already populated economy store may rebuild indexes on its first startup. Start the service separately and wait for its listening message before opening the client; do not interrupt that migration. Later starts reuse the persisted indexes and category catalog.

Published dumps provide daily stash estimates, not 15-minute backfill, exchange volume, or listing/sample counts. Only Chaos-quoted currency rows are imported; differently directed quotes are explicitly counted as exclusions, not silently inverted or merged. Unknown historical categories remain browsable under Historical. Original link ranges such as `1-4 links` are kept as ranges, not treated as an exact four-link price. Dumps do not become Gem flipping snapshots: their confidence/variant coverage cannot reconstruct a complete same-instant gem/material bundle.

**Attribution:** historical data is provided by poe.ninja under the **CC BY-SA** license stated on its [data page](https://poe.ninja/poe1/data). Import records retain source URLs, checksums, license attribution, coverage, counts and exclusions. Retain the source attribution and applicable share-alike terms when redistributing imported data. Faustus's own source-code license is unchanged.

The Gem flipping page compares poe.ninja SkillGem variants by level, quality, and corruption. Its shared search filters all three strategies, and clicking a gem row carries that gem into the search when switching strategies. The **As of** selector defaults to **Latest**, followed by locally saved instants, newest first. Buy, sell, confidence counts, and automatic material prices all come from the selected snapshot, not different sparkline endpoints; manual material overrides are optional. Each snapshot preserves the latest known gem and currency overviews together, including their separate fetch times; the feeds are not necessarily updated simultaneously. The Current prices tab always uses current data, even when viewing a saved gem instant. Changing leagues resets the instant to Latest; refreshing the same league preserves a pinned instant.

The strategies estimate level-1-to-max flips, 20% quality upgrades using 20 Gemcutter's Prisms, and approximate expected Vaal corruption returns. Facetor's Lens is an optional, manually entered Chaos cost in Level Flip, including when viewing a saved instant. Its price defaults to 50 Chaos, with one Lens per flip and no quantity control. **Min** in the compact confidence controls configures the sample/listing threshold (default 10, a positive whole number): buy and all valued sell variants must have at least that many samples and listings. Missing counts are low confidence. Low-confidence gems are excluded by default; **Low conf** includes them with a LOW CONF badge. Invalid thresholds show an error instead of silently using another value. Lens XP varies and is not included in the gem feed. Estimates exclude trade fees, leveling time, and market slippage.

**Vaal corrupt** uses an explicitly simplified four-bucket model on a max-level, 20%-quality uncorrupted gem: +1 level (12.5%), +3 quality (12.5%), max-level Q20 corrupted (62.5%), and max-level Q0-19 corrupted (12.5%). The -1 level outcome is valued after releveling to max. This approximation groups 21-22% quality with +3 quality and treats Vaal conversion as unchanged; it is not an exact mechanics simulation, including for special gems. +1 level and unchanged use their exact corrupted variants. +3 quality prefers Q23, falling back to the lowest listed Q21-23 price; reduced quality uses the lowest listed Q0-19 price. Bucket estimates are flagged in the breakdown. An entirely unpriced bucket excludes the gem; uncorrupted prices and zero valuations are never substituted. The Sell column becomes **EV** (expected value) on this tab: the probability-weighted resale value. Net subtracts the buy price and one Vaal Orb; ROI divides Net by that total cost. Hover or focus EV, Net, or ROI for a table of outcome probabilities, resale valuations, weighted contributions, and summed EV, followed by cost, Net, and ROI formulas and the model assumptions. Other strategies retain the Sell label. Listings shows buy listings / minimum listings across the valued outcomes, with confidence checked on every valued variant.

The breakdown table includes the listing count for each outcome's valued variant, including estimated bucket representatives; missing counts display N/A rather than zero. EV, Net, and ROI expose these details on hover or focus without a separate explanatory label above the table.

The Vaal tab also includes a sortable **Max value** column: the highest resale valuation among the modeled outcomes, before costs and without probability weighting.

GCP and Vaal Chaos prices are editable on the gem page. Empty fields use the selected instant's feed prices, shown as placeholders; typing a value overrides that price immediately. Manual overrides accept non-negative finite prices, including zero, and persist across strategies, snapshots, and refreshes within a league. Clear a field to restore its feed price; changing leagues resets both overrides. Manual prices do not change saved snapshots or the Current prices tab. Invalid values show an error instead of silently falling back; explicit overrides also work when material feed prices are unavailable.

The service also exposes:

- `GET /health`
- `GET /api/v1/leagues` (cached poe.ninja economy league options)
- `POST /api/v1/selection` with `{"id":"<league-id>"}`
- `GET /api/v1/status`
- `GET /api/v1/economy/categories` (supported live categories plus imported historical-only categories)
- `GET /api/v1/economy/leagues` (live leagues and imported league availability)
- `GET /api/v1/economy/overview?league=LEAGUE_ID&category=Currency&source=exchange` (or `source=stash`, optionally `date=YYYY-MM-DD`)
- `POST /api/v1/economy/refresh?league=LEAGUE_ID&category=Currency&source=exchange` (provider-cache-aware revalidation)
- `GET /api/v1/economy/dates?league=LEAGUE_ID&category=Currency&source=stash`
- `GET /api/v1/economy/history?league=LEAGUE_ID&category=SkillGem&source=stash&item=ITEM_ID` (optionally `days=7`, `30`, etc.; up to 20,000 newest observations)
- `GET /api/v1/economy/archives` (cached published catalog and persisted import status; `refresh=true` rediscovers availability)
- `POST /api/v1/economy/import` with `{"name":"Ancestors"}` (queues an explicit softcore-only import)
- `GET /api/v1/exchange/prices?league=LEAGUE_ID&type=Currency` (also supports `Fragment`; responses are cached for 15 minutes and revalidated with ETags)
- `GET /api/v1/gems?league=LEAGUE_ID` (poe.ninja SkillGem variants, prices, listing counts, and sparkline history; cached for five minutes)
- `GET /api/v1/gem-analysis?league=LEAGUE_ID` (latest gem/material data, explicit independent provider errors, and saved snapshot options; records a snapshot when gem data is available)
- `POST /api/v1/gem-analysis/refresh?league=LEAGUE_ID` (revalidates both feeds with ETags and saves the resulting bundle; concurrent/repeated requests for a league reuse the last refresh attempt for one minute, including explicit provider errors)
- `GET /api/v1/gem-analysis?league=LEAGUE_ID&snapshot=SNAPSHOT_ID` (immutable saved gem/material data for that league; database-only, without contacting providers; missing or cross-league IDs return 404)
- `GET /api/v1/exchange/markets?league=LEAGUE_NAME&limit=100`

The market query returns the latest stored snapshot for each pair in each league. The raw ratio and volume fields remain keyed by item identifier; the app does not infer a buy/sell price from them yet.

## Build, Test, and Deploy the UI

Run these commands from `crates\market-ui`:

```powershell
npm test                    # Analysis, API transport, and dashboard state tests
npm run build               # Type-check and create standalone web assets in dist
npx playwright install chromium   # Once, for browser tests
npm run test:e2e            # Tests the production web build; mocks provider data
npm run tauri -- build      # Builds desktop assets, executable, and Windows NSIS installer
```

The web build defaults to same-origin `/api/v1/...` requests. Deploy `dist` as static assets and reverse-proxy `/api` to the market service under the same HTTPS origin. It has no dependency on a Tauri runtime. Set `VITE_API_BASE_URL=https://your-api.example` at build time only when hosting the API on a different origin, and allow the frontend's exact origin on the service:

```powershell
cargo run -p market-service -- serve --allow-origin https://your-frontend.example
```

`--allow-origin` can be repeated; supplying it replaces the default desktop origins (`tauri://localhost`, `http://tauri.localhost`, and `https://tauri.localhost`). CORS permits GET/POST and JSON Content-Type, without credentials or wildcard origins. Same-origin web requests and Vite's development proxy do not require extra CORS origins.

Desktop packaging uses `.env.desktop` to point directly at `http://127.0.0.1:8787`. Override `VITE_API_BASE_URL` when building for another service, and also update `connect-src` in `src-tauri/tauri.conf.json` to allow that exact service origin. No Tauri command capabilities are granted. A remote API should use HTTPS. The service is still unauthenticated and its collector selection is global: CORS is not authentication, and a public multi-user deployment needs access control and a decision about collector ownership before exposing selection or ingestion.

The GPUI crate and its tests remain available:

```powershell
cargo run -p market-ui-deprecated --bin faustus-desktop-deprecated
cargo test -p market-ui-deprecated
```

The replacement implements tested TypeScript gem calculations, including same-instant snapshots, configurable confidence filtering, material costs, sorting, the 100-result cap, and 50-row batches. The sortable Type column and shared type filter use one category per gem: Skill, Support, Exceptional, Transfigured, or Trarthus. Special categories come from poe.ninja's `metadata.gemKind`, exposed by the service as `gem_kind`, and take precedence over Skill/Support. Untagged names ending in " Support" are Support; other untagged gems are Skill. Unrecognized provider categories appear as Unknown rather than being guessed from gem names. Type, search, and confidence settings persist across strategies, instants, and refreshes and are applied before the 100-result cap. Tables use native browser scrolling, including middle-button autoscroll where supported by the browser/platform. Search persists across gem strategies; clicking a gem name selects that name as the search.

The Type dropdown is a multi-select list: click a type to toggle its highlight, without checkboxes. The exclusive **All** option is selected by default, while individual types are unselected. Choosing a type deselects All; choosing All clears individual selections and includes every category (including Unknown). With individual types selected, a gem matches any of them. Deselecting every individual type shows no opportunities. Its fixed-width summary shows All, None, one selected name, or a count; the tooltip retains the full selection. Selections persist across strategies, snapshots, and refreshes. The dropdown supports keyboard navigation, Escape to close, and clicking or tabbing outside to dismiss. The date dropdown uses compact Canadian date labels such as `Nov 9, 19:05` in the browser's local timezone, with full local dates and timezone names in option tooltips. Stored timestamps and snapshot ordering remain unchanged.

All UI dates use Canadian English (`en-CA`) formatting and the browser's local timezone. Full timestamps use year-month-day dates, 24-hour time, and a timezone name; snapshot options use an abbreviated month. Latest includes the latest gem overview's fetch time (for example, `Latest 19:56`), with its full local timestamp in the tooltip, even when viewing a saved instant. The gem page omits strategy descriptions and the data/explanation caption; calculation caveats remain documented above. API and database timestamps remain unchanged.

The Aurelia application registers `MarketApi` as a container-scoped singleton. `MarketApp` obtains it with `resolve(MarketApi)`; tests can override that registration and construct the app through a DI container.

Frontend source organization:
- `src/pages/current-prices/` contains the economy workspace, item details, charts and data-manager UI; `src/pages/gem-flipping/` contains the gem page template. The old GGG template is no longer imported.
- `src/core/` contains the HTTP client, API types, gem-analysis calculations, and their unit tests.
- `src/` retains the entry point, environment declarations, app shell/shared dashboard state, app tests, and shared styles. Gem flipping receives shared state through a `model` binding; the economy component owns separate filters/history state and receives the live league and refresh token. Switching tabs preserves economy context.

## Data Collection Notes

Normal service operation no longer starts the GGG historical cursor collector. Its old records and implementation remain available for deprecated integrations, without deleting or rewriting their data.

poe.ninja's [supported economy API](https://poe.ninja/docs/api) supplies category overviews, not a supported full item-detail history API. The service records applicable categories for the selected live league on a 15-minute cycle with staggered requests. Provider responses are shared by economy and gem/material consumers, cached by league/category/source and conditionally revalidated with ETags. Manual refresh respects provider cache lifetimes; 304 responses retain original source timestamps instead of manufacturing new observations. Economy API requests go through the backend, not from end-user browsers. Item images load directly from `https://web.poecdn.com`; the desktop CSP permits that image origin only, without expanding script or API access. Send an appropriate descriptive User-Agent using `--user-agent`.

Gem-analysis snapshots are recorded in SQLite when the dashboard loads or requests Latest. An independent server collector also revalidates the selected league's gem and material feeds immediately after league selection and every 15 minutes thereafter, without needing an open client or waiting for the GGG exchange backlog. Collection switches with the selected league and continues while the service runs; after a service restart it waits for a client to select a league. Failures are logged and retried on the next interval. There is no historical backfill.

The client's **Refresh** button requests cache-aware revalidation; a shared provider response stays cached until its cache lifetime allows another conditional request. Gem snapshot capture also has a one-minute per-league cooldown, including failed attempts, and serializes captures to prevent duplicate refreshes. ETags are retained; an unchanged provider response keeps its original source timestamp and identical bundles reuse their existing snapshot rather than manufacturing a new instant. A pinned historical instant stays selected on Refresh, while Current prices and the latest snapshot are updated separately.

The selector lists the 200 most recent saved instants (plus a pinned older instant, if needed); older records remain stored. Snapshots survive service restarts. Previously collected Currency/Fragment stash history cannot reconstruct historical gem variants. The [documented poe.ninja API](https://poe.ninja/docs/api) only exposes current overviews, and its league list supplies no league-start timestamps, so saved instants use actual capture timestamps displayed in local time rather than invented Hour/Day/Week labels. If material prices failed at capture time, that failure is preserved and material-dependent strategies stay unavailable unless a manual material price is entered; current material prices are never substituted automatically. Restart the market service after updating to enable periodic gem collection and the explicit refresh endpoint.

Only the deprecated GPUI WASM frontend needs nightly because GPUI Kit's web backend enables `wasm_thread`. The Aurelia web frontend uses the standard Vite build and does not compile Rust to WebAssembly.

GGG's API and poe.ninja's economy API can change without notice. Respect their rate limits and caching guidance. poe.ninja permits use of its documented economy endpoints but prohibits directly replicating its site; this app is intended for cross-source market analysis. Do not use undocumented site endpoints.

This product isn't affiliated with or endorsed by Grinding Gear Games in any way.

## Workspace

- `market-core`: shared identifiers, exact decimal quotes, comparisons, and API types.
- `faustus-fetch`: GGG cursor-page client and raw response preservation.
- `ninja-fetch`: documented economy client, ETag handling, and overview DTOs.
- `market-store`: SQLite migrations, transactional cursor recording, economy/import history, and gem snapshots.
- `market-service`: local economy HTTP API, scheduled collectors, and ingestion/import CLI.
- `market-ui`: Aurelia v2/Vite frontend in `src`, with the Tauri v2 Rust desktop shell in `src-tauri`.
- `market-ui-deprecated`: preserved GPUI desktop and single-threaded WebAssembly frontends; major pages live under `src/pages/`.

The deprecated frontends use GPUI Kit 0.9.1, which pins the matching GPUI 0.3.12 snapshot and platform crates. Rust 1.92 or newer is required by the current dependency graph.

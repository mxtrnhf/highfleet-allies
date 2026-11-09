# highfleet-allies

Two mods for **HighFleet (Steam 1.163)**, built on [logdot's Highfleet-Modloader](https://github.com/logdot/Highfleet-Modloader):

| Mod | What it does |
|---|---|
| [`ally/`](ally) (**hf_ally**) | An AI-flown **wingman ship** that fights beside you, pre-combat **ALLY** buttons with stock click sounds, language-aware radio lines (`RADIO`/`RADIO_ENG`), ally **battle honor/XP sharing** (`ally_honor_share`), retreat/fuel logic, and an optional **Zenith hold-fire** (`ally_zenith_autofire`); **carrier plane armament buttons** (bombs, rockets, optional K-13 AA missiles, charged to your inventory; they follow the game's own plane and loadout tables, so aircraft added by other mods through the QoL `planes` option get a button too); **enemy group size** (`enemy_group_size`, enemy waves); the Sprint-intercept fix. |
| [`qol/`](qol) (optional) | Logdot's QoL `main` **plus manual editor elevation and optional gun/KAZ firing-arc rules**. **Not required**: logdot's stock QoL release works with `hf_ally`/`hf_ai`. Our build includes refuel/speed settings, manual elevation ([PR #20](https://github.com/logdot/highfleet-qol/pull/20)) and `elevation_rules` ported from our separate fork (vanilla by default). |
| [`ai/`](ai) (**hf_ai**) | A tactical **AI layer**: enemies and allies dodge hostile aim lines, deny the dive, drop flares and evade missiles, and **empty their missile racks** in a burst (`ai_missile_dump_*`). Plain-language guide: [docs/hf-ai-tuning-guide.md](docs/hf-ai-tuning-guide.md). |

Both are optional, independent DLLs. Everything is reverse-engineered, so **Steam 1.163 only**; they refuse to start on other versions. Back up your `Saves` folder first.

> **PACK v1.0.3 — IMPROVED DEBUG OVERLAY**
> **From v0.15.0 or any later development build:** replace both `Modloader/config/ally.json` and `Modloader/config/hf_ai.json` with the v1.0.0 presets. In `ally.json`, `enabled` is the only master switch; `auto_retreat` and `enemy_all_at_once` are retired; `spawn_ally`, `ally_toggle`, `ally_reticles`, `max_enemy_ships`, `plane_bombers`, `marker_seam`, `marker_delay_ms` and the retired logging keys are ignored. In `hf_ai.json`, the retired curtain keys (`curtain_*` / `launch_flares_*`) and early-fire keys (`aim_fire_*`) do nothing, and `dive_dwell_s` defaults to `0.3`. Ally radio lines follow the game language (`Media/Snd/RADIO` in Russian, `Media/Snd/RADIO_ENG` in English); custom voice paths and custom audio are planned for a future version.
> **From v0.14.13 or older:** replace both configs; the v1.0.0 presets include the earlier grouped/HEX/logging cleanups.

Release pack **v1.0.3** contains **hf_ally v1.0.3**, **hf_ai v1.0.0** and the bundled QoL build ([ally changelog](ally/CHANGELOG.md), [ai changelog](ai/CHANGELOG.md)). The improved debug overlay was confirmed working in game on 2026-11-09.

## Install (players)
1. Install the [Modloader](https://github.com/logdot/Highfleet-Modloader/releases) (rename the original `bass.dll` as its README says).
2. Download the pack from [Releases](../../releases) and extract the zip directly into the game folder (the one with `Highfleet.exe`); it contains `Modloader/mods`, `Modloader/config` and `Media`.
3. **QoL mod.** The mods need logdot's [highfleet-qol](https://github.com/logdot/highfleet-qol) for its **flare crash fix** (a 1.163 crash when a proximity-fused missile meets a flare; `hf_ai` drops far more flares than the stock game). **His stock release (v1.3.0) works fine with our mods**: if you already run it, keep it. The pack also ships our build, `Modloader/mods/highfleet_qol.dll`, built from the [`qol/`](qol) folder: logdot's current `main` plus our manual elevation. It adds only features **that are not in his latest release yet**: refuel rate and speed curve (merged into his `main`, no release since) and elevation (our [pull request #20](https://github.com/logdot/highfleet-qol/pull/20), still a draft). It replaces his DLL if you copy it over; once he cuts a release with these, use his release and drop ours. Keep `enable_flare_crash_fix` on (the default). `hf_ai` logs a warning at start-up if QoL is missing or set up wrongly. Use only **one** `hf_ally` DLL in `Modloader/mods`.
4. Settings are JSON files in `Modloader/config/` (`ally.json`, `hf_ai.json`), written with the checked-in player defaults on first run. `"enabled": false` turns a mod off.

## Settings

The player defaults are version-controlled in [`ally/config/ally.json`](ally/config/ally.json)
and [`ai/config/hf_ai.json`](ai/config/hf_ai.json). Release files and compiled
missing-key defaults match those presets. See [player-defaults.md](docs/player-defaults.md)
for the profile and release policy. Existing configs keep their explicit values.

Every setting is already in the config files of the pack, so there is nothing to discover: [`docs/ally-config.md`](docs/ally-config.md) (hf_ally, every key), [`docs/hf-ai-config-reference.md`](docs/hf-ai-config-reference.md) (hf_ai, every key) with the plain-language [`docs/hf-ai-tuning-guide.md`](docs/hf-ai-tuning-guide.md), and [`docs/qol-guide.md`](docs/qol-guide.md) (refuel, speed formulas and manual elevation, for the QoL build).

**Combat ship switching:** **1** takes the top ally-panel ship, **2** the bottom,
and **TAB** cycles deployed friendly ships. Your old ship becomes an AI ally.
Enabled by `ship_switching` (default true); [details and keybinding caveats](docs/ship-switching.md).

**Debug overlay:** its own `"debug_overlay": { "enable": true }` block in `ally.json`
draws labelled rings around every ship (radii set by `circles`), a label with its native
AI state, the AI steering waypoint with its distance (`ai_waypoint`), and optional lead
markers showing where to aim to hit each hostile ship, with a small "put guns here"
bearing circle at `pointer_length` world units from your hull (`prediction_marker`).
`pointer_length` also sets the ordinary native aim-line length (default 300 units); with
hf_ai v0.10.9+ it adds hf_ai's tactics/missile-evasion state. Off by default;
[details](docs/debug-overlay.md).

The bundled QoL also reads top-level `elevation_rules`: `vanilla` (default),
`modern`, `classic` or `classic_kaz`. This controls gun/KAZ firing arcs independently
of manual editor elevation. See [QoL guide](docs/qol-guide.md#elevation_rules-gunkaz-firing-arcs).

The stock aircraft AI fix and null-fleet/vector guards now live in bundled QoL,
not ally. Their `fix_plane_ai_crash` and `fix_merge_crash` keys belong in `qol.json`
and default to true. Update both DLLs together. Stock upstream QoL does not contain
these two additions; the fleet guards are not a confirmed fix for every merge crash.

## Build (developers)
Rust nightly/stable ≥ 1.88 with `cargo-xwin` (cross-compiles to Windows MSVC from Linux; works on Windows too):

    cd ally && cargo xwin build --release --target x86_64-pc-windows-msvc
    cd ai   && cargo xwin build --release --target x86_64-pc-windows-msvc

Logging settings are grouped under `"logging"` in each mod's JSON. In `ally.json`, set `"logging": { "debug_log": true }` for a verbose megalog, watchdog and crash dump (`Modloader/logs/hf_ally_megalog_*.log`, `hf_ally_crash_*.dmp`; dumps may contain paths and campaign data, delete them after sending). `debug_log` is an on/off switch and defaults to `false`; the legacy `megalog` cargo feature does not force it on. `logging.gui_inspect` and `logging.log_creatures` remain available (both default false), as does `hf_ai.json`'s `logging.log_panic`. Old top-level forms of the kept keys remain readable; explicit nested settings take precedence. `max_lines`, `log_precombat`, `probe_lines`, `log_colors` and `dead_node` are retired and ignored. See [retired diagnostics](docs/retired-diagnostics.md) and [all ally settings](docs/ally-config.md). Ship/battle CSV capture, F9 capture markers and replay-analysis tools have been removed, including the `telemetry` cargo feature.
Build and publish releases through [GitHub Actions](.github/workflows/release.yml): run **Build and publish release** with the pack version. It builds all three DLLs on Windows, packages the tracked presets, and publishes the ZIP to GitHub Releases. No separate pack checksum is published. For local packaging only: `python3 tools/package_release.py --version 1.0.3 --validation "Debug overlay confirmed working in game (2026-11-09)." --output ../release_packs`.

Automated tests have been removed. Build success does not validate hooks or gameplay; validate those in game.

### Ally source ownership

`ally/src/lib.rs` is the DLL composition root, not a shared state container. Configuration
belongs to `config`; pre-combat buttons, cached GUI addresses and complete-list validation
belong to `precombat`; battle arming, tracked allies, deployment and retreat belong to
`battle`. `ally_ammo` owns doctrine, funding records and refunds. `sound` retains native
sound-status storage, while `battle_diagnostics` owns callback-local probes and tracking.
Shared native addresses and scalar memory helpers live in `native`, alongside the separate
`native_layout` contracts. `bootstrap` coordinates installation using one `PatchSession`;
feature owners contribute their patches before the final permanent install.

Keep state with the operations that enforce its lifetime rules. A cached native address is
not proof of a live object; moving code must preserve membership guards, bounded traversal,
native ownership, callback order and blocking versus non-blocking lock behavior.

## Releases and versioning
Before 1.0, `hf_ally` uses `0.MINOR.PATCH`: **PATCH** for small fixes within a feature line, **MINOR** for a new feature line or a notable public behavior/config change. The release pack has its own `vX.Y.Z` tag and normally mirrors the ally version; `hf_ai` and QoL keep independent component versions. The GitHub Actions release workflow takes the pack version as input, builds the three DLLs on Windows, stamps `@PACK_VERSION@`, `@ALLY_VERSION@` and `@AI_VERSION@` into the notes/README, and publishes the ZIP. See `manifest.json` in a pack for the exact component versions and source commit.

## How it works / research notes
The game's addresses, struct layouts and findings are in a separate repo: **[mxtrnhf/hf_notes](https://github.com/mxtrnhf/hf_notes)** (Ghidra notes on 1.163, battle AI, missiles, planes, pre-combat screen). Source comments cite these notes by file name.

## Credits and legal
* Built on logdot's [Highfleet-Modloader](https://github.com/logdot/Highfleet-Modloader), [patchy-rs](https://github.com/logdot/patchy-rs), [highfleet-qol](https://github.com/logdot/highfleet-qol) and [highfleet-rs](https://github.com/logdot/highfleet-rs); the `ai_flares` and missile-dump ideas started in the QoL fork.
* The combat ship-switching idea was inspired by **greatf1y** on Discord.
* `hf_ally` and `hf_ai` pin `highfleet-rs` to revision `05c6207` for the Steam 1.163 aircraft/loadout, Body, AircraftLogic, PartStats, Creature and GUI layouts. Compile-time checks preserve the existing field offsets; game-native constructors, string assignment and link operations still manage game-owned objects. Fields upstream does not model yet stay local with their raw offsets.
* Much of the code and notes were written with the help of Claude Code (Anthropic); commits carry the co-author line.
* Unofficial fan project, not affiliated with the HighFleet developers or publisher. No game files are included. MIT licensed, see [LICENSE](LICENSE).

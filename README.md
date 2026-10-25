# highfleet-allies

Two mods for **HighFleet (Steam 1.163)**, built on [logdot's Highfleet-Modloader](https://github.com/logdot/Highfleet-Modloader):

| Mod | What it does |
|---|---|
| [`ally/`](ally) (**hf_ally**) | An AI-flown **wingman ship** that fights beside you, pre-combat **ALLY** buttons, radio lines, retreat/fuel logic; a **carrier plane armament button** (bombs, rockets, optional K-13 AA missiles, charged to your inventory); **more enemies per battle** (`max_enemy_ships`, enemy waves); crash guards for three vanilla crashes; the Sprint-intercept fix. |
| [`qol/`](qol) (optional) | A copy of logdot's QoL `main` **plus manual elevation** in the ship editor. **Not required**: logdot's stock QoL release works with `hf_ally`/`hf_ai`. Ours only adds what his release does not have yet (refuel, speed, elevation) until he merges [our pull request](https://github.com/logdot/highfleet-qol/pull/20) and cuts a release. |
| [`ai/`](ai) (**hf_ai**) | A tactical **AI layer**: enemies dodge your aim line, deny the dive, drop flare curtains and evade missiles (allied AI ships too), and **empty their missile racks** in a burst (`ai_missile_dump_*`). Plain-language guide: [docs/hf-ai-tuning-guide.md](docs/hf-ai-tuning-guide.md). |

Both are optional, independent DLLs. Everything is reverse-engineered, so **Steam 1.163 only**; they refuse to start on other versions. Back up your `Saves` folder first.

## Install (players)
1. Install the [Modloader](https://github.com/logdot/Highfleet-Modloader/releases) (rename the original `bass.dll` as its README says).
2. Download the pack from [Releases](../../releases) and extract the zip directly into the game folder (the one with `Highfleet.exe`); it contains `Modloader/mods`, `Modloader/config` and `Media`.
3. **QoL mod.** The mods need logdot's [highfleet-qol](https://github.com/logdot/highfleet-qol) for its **flare crash fix** (a 1.163 crash when a proximity-fused missile meets a flare; `hf_ai` drops far more flares than the stock game). **His stock release (v1.3.0) works fine with our mods**: if you already run it, keep it. The pack also ships our build, `Modloader/mods/highfleet_qol.dll`, built from the [`qol/`](qol) folder: logdot's current `main` plus our manual elevation. It adds only features **that are not in his latest release yet**: refuel rate and speed curve (merged into his `main`, no release since) and elevation (our [pull request #20](https://github.com/logdot/highfleet-qol/pull/20), still a draft). It replaces his DLL if you copy it over; once he cuts a release with these, use his release and drop ours. Keep `enable_flare_crash_fix` on (the default). `hf_ai` logs a warning at start-up if QoL is missing or set up wrongly. Use only **one** `hf_ally` DLL in `Modloader/mods`.
4. Settings are JSON files in `Modloader/config/` (`ally.json`, `hf_ai.json`), written with defaults on first run. `"enabled": false` turns a mod off.

## Build (developers)
Rust nightly/stable ≥ 1.88 with `cargo-xwin` (cross-compiles to Windows MSVC from Linux; works on Windows too):

    cd ally && cargo xwin build --release --target x86_64-pc-windows-msvc
    cd ai   && cargo xwin build --release --target x86_64-pc-windows-msvc

`ally` has a runtime switch for bug reports: set `"debug_log": true` in `ally.json` to write a verbose megalog, run a watchdog and save a crash dump (`Modloader/logs/hf_ally_megalog_*.log`, `hf_ally_crash_*.dmp`; the dump may contain paths and campaign data, delete it after sending). The cargo feature `megalog` turns the same mode on in the build itself, and `telemetry` adds battle CSVs for AI research.
Tests for the pure math: `cd ai && cargo xwin test --release --target x86_64-pc-windows-msvc` (needs wine).

## How it works / research notes
The game's addresses, struct layouts and findings are in a separate repo: **[mxtrnhf/hf_notes](https://github.com/mxtrnhf/hf_notes)** (Ghidra notes on 1.163, battle AI, missiles, planes, pre-combat screen). Source comments cite these notes by file name.

## Credits and legal
* Built on logdot's [Highfleet-Modloader](https://github.com/logdot/Highfleet-Modloader), [patchy-rs](https://github.com/logdot/patchy-rs) and [highfleet-qol](https://github.com/logdot/highfleet-qol); the `ai_flares` and missile-dump ideas started in the QoL fork.
* Much of the code and notes were written with the help of Claude Code (Anthropic); commits carry the co-author line.
* Unofficial fan project, not affiliated with the HighFleet developers or publisher. No game files are included. MIT licensed, see [LICENSE](LICENSE).

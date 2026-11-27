# HighFleet Allies

Four independently loadable mods: campaign wingmen and ship switching, tactical
combat AI for both sides, Shipworks custom encounters, and campaign shelling.

**HighFleet Steam 1.163 only · Split builds require [Modloader v4.2.0-preview.1](https://github.com/logdot/Highfleet-Modloader/releases/tag/v4.2.0-preview.1)**

**[Download latest release](https://github.com/mxtrnhf/highfleet-allies/releases/latest)** ·
[All releases](https://github.com/mxtrnhf/highfleet-allies/releases) ·
[Report an issue](https://github.com/mxtrnhf/highfleet-allies/issues)

<!-- Add Highlights here. -->

## Install

1. Close the game and back up your `Saves` folder. If upgrading, also back up
   `Modloader/config/` before extracting the ZIP.
2. For a split build, install [Highfleet-Modloader v4.2.0-preview.1](https://github.com/logdot/Highfleet-Modloader/releases/tag/v4.2.0-preview.1).
3. Download the **`hf_release_pack_*.zip`** asset (not the GitHub source archives)
   and extract it into the folder containing `Highfleet.exe`.
4. Check `Modloader/mods/`: keep only one version of each mod you want to play.
   Remove the pre-split `hf_ally` DLL before enabling ACES or CCU.
5. If upgrading, merge your custom settings into the new `ally.json`,
   `hf_ai.json`, and `qol.json` presets. Move `crash_button` from `ally.json` to
   `aces.json` if you use it. CCU is enabled by its own config; set `"enable": false`
   in `ccu.json` to retain the old shelling-off default. See the
   [player defaults and legacy settings](docs/player-defaults.md) if coming from
   a pre-1.0 build.

**Which QoL DLL?** Use the bundled `hf_qol_v*.dll` for a fresh install. It
includes aircraft and fleet crash guards not in the stock upstream release. If
you prefer to keep [upstream QoL](https://github.com/logdot/highfleet-qol/releases),
extract the ZIP to a temporary folder and keep your existing QoL DLL and config
instead of copying the bundled ones. Combat AI works without QoL. Enable
`enable_flare_crash_fix` in `Modloader/config/qol.json` to use automatic flares.

## Settings and controls

Edit settings in `Modloader/config/` while the game is closed. Set
`"enabled": false` in `ally.json` or `hf_ai.json`, or `"enable": false` in
`aces.json` or `ccu.json`, to turn off that mod.

- **Allies (`ally.dll`):** [Player manual V2 (PDF)](docs/Allies_Manual_V2.pdf) · [Settings reference](docs/ally-config.md) · [Ship switching](docs/ship-switching.md)
- **Combat AI:** [Tuning guide](docs/hf-ai-tuning-guide.md) · [Settings reference](docs/hf-ai-config-reference.md)
- **ACES (`aces.dll`):** [Shipworks rosters and A.C.E.](docs/shipworks-test-allies.md)
- **CCU (`ccu.dll`):** [Campaign shelling](docs/strategic-shelling.md)
- **QoL:** [Crash fixes, refueling, map speed, and editor settings](docs/qol-guide.md)

These guides track `main` and may cover changes that aren't in the latest ZIP.
Check the [release notes](https://github.com/mxtrnhf/highfleet-allies/releases) for
what actually shipped, or use the guides included in your ZIP.

## Troubleshooting

First check that you're on Steam 1.163 and have no duplicate mod DLLs. If the
problem persists, [open an issue](https://github.com/mxtrnhf/highfleet-allies/issues)
with your pack version, other installed mods, steps to reproduce, and the newest
`Highfleet_*.log` from `Modloader/logs/`. See the
[logging guide](docs/ally-config.md#logging-logging-object) for verbose ally logs
and crash dumps. Review logs before sharing; dumps may contain local paths and
campaign data.

## Development

[Build and release guide](docs/development.md) · [Split-mod test matrix](docs/split-mod-test-matrix.md) ·
[Allies](ally/CHANGELOG.md) · [AI](ai/CHANGELOG.md) · [ACES](aces/CHANGELOG.md) ·
[CCU](ccu/CHANGELOG.md) · [QoL](qol/CHANGELOG.md)

## Credits and license

Built on logdot's [Highfleet-Modloader](https://github.com/logdot/Highfleet-Modloader),
[patchy-rs](https://github.com/logdot/patchy-rs), [highfleet-rs](https://github.com/logdot/highfleet-rs),
and [highfleet-qol](https://github.com/logdot/highfleet-qol). Combat ship switching
was inspired by **greatf1y** on Discord. Parts of the code and notes were developed
with Claude Code.

Unofficial fan project, not affiliated with the developers or publisher of
HighFleet. No base-game executable is included. [MIT license](LICENSE).

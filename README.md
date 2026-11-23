# HighFleet Allies

Bring AI wingmen into campaign battles, switch between friendly ships mid-fight,
and build custom ally and enemy rosters in Shipworks. The pack also adds tactical
combat AI for both sides.

**HighFleet Steam 1.163 only · Requires Modloader v4**

**[Download latest release](https://github.com/mxtrnhf/highfleet-allies/releases/latest)** ·
[All releases](https://github.com/mxtrnhf/highfleet-allies/releases) ·
[Report an issue](https://github.com/mxtrnhf/highfleet-allies/issues)

<!-- Add Highlights here. -->

## Install

1. Close the game and back up your `Saves` folder. If upgrading, also back up
   `Modloader/config/` before extracting the ZIP.
2. Install [Highfleet-Modloader v4](https://github.com/logdot/Highfleet-Modloader#instalation)
   if it isn't already installed.
3. Download the **`hf_release_pack_*.zip`** asset (not the GitHub source archives)
   and extract it into the folder containing `Highfleet.exe`.
4. Check `Modloader/mods/`: remove older `hf_ally` and `hf_ai` DLLs so only one
   of each remains. Keep only one QoL DLL as well.
5. If upgrading, merge your custom settings into the new `ally.json`,
   `hf_ai.json`, and `qol.json` presets rather than restoring old files wholesale. See the
   [player defaults and legacy settings](docs/player-defaults.md) if coming from
   a pre-1.0 build.

**Which QoL DLL?** Use the bundled `hf_qol_v*.dll` for a fresh install. It
includes aircraft and fleet crash guards not in the stock upstream release. If
you prefer to keep [upstream QoL](https://github.com/logdot/highfleet-qol/releases),
extract the ZIP to a temporary folder and keep your existing QoL DLL and config
instead of copying the bundled ones. With either build, keep
`enable_flare_crash_fix` enabled in `Modloader/config/qol.json` when using `hf_ai`.

## Settings and controls

Edit settings in `Modloader/config/` while the game is closed. Set
`"enabled": false` in `ally.json` or `hf_ai.json` to turn off that mod.

- **Allies:** [Player manual V2 (PDF)](docs/Allies_Manual_V2.pdf) · [Settings reference](docs/ally-config.md) · [Ship switching](docs/ship-switching.md) · [Shipworks test battles](docs/shipworks-test-allies.md)
- **Combat AI:** [Tuning guide](docs/hf-ai-tuning-guide.md) · [Settings reference](docs/hf-ai-config-reference.md)
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

[Build and release guide](docs/development.md) · [Ally changelog](ally/CHANGELOG.md) ·
[AI changelog](ai/CHANGELOG.md) · [QoL changelog](qol/CHANGELOG.md)

## Credits and license

Built on logdot's [Highfleet-Modloader](https://github.com/logdot/Highfleet-Modloader),
[patchy-rs](https://github.com/logdot/patchy-rs), [highfleet-rs](https://github.com/logdot/highfleet-rs),
and [highfleet-qol](https://github.com/logdot/highfleet-qol). Combat ship switching
was inspired by **greatf1y** on Discord. Parts of the code and notes were developed
with Claude Code.

Unofficial fan project, not affiliated with the developers or publisher of
HighFleet. No base-game executable is included. [MIT license](LICENSE).

# Retired ally diagnostics

This page keeps the history of diagnostic features that are no longer part of
`hf_ally`. The source files are preserved under
[`ally/src/retired/`](https://github.com/mxtrnhf/highfleet-allies/tree/main/ally/src/retired)
but are **not compiled** into the mod.

## `logging.dead_node` (retired in v0.14.19)

The dead-node diagnostic was a read-only text trace of the native
`Collect_dead ... m_master` assertion. It installed two byte-validated hooks in
the Steam 1.163 collector:

| Site | Original bytes | Purpose |
|---|---|---|
| `0x1401be274` | `48 83 bf 88 00 00 00 00` | node path |
| `0x1401be2f8` | `48 83 7f 58 00` | joint path |

Just before the game's fatal assertion, the hooks wrote class/RTTI, state, ids,
master, name, raw bytes and the owning ship part to
`Modloader/logs/hf_ally_deadnode.log` and the normal log. They did not suppress
the assertion or change game state.

The config key was removed because the feature had served its investigation
purpose. The original implementation remains at
[`ally/src/retired/dead_node.rs`](https://github.com/mxtrnhf/highfleet-allies/blob/main/ally/src/retired/dead_node.rs).

## `logging.log_colors` (retired in v0.14.19)

The colour probe logged every distinct colour passed to the game's sprite-colour
setter (`0x140172ae0`), vector-colour setter (`0x140016fe0`) and font style call
(`0x1401a8710`), with the caller return address. It also ran a `TITLE` callout
probe every 2 s during battle. It was used to read off the HUD's friendly blue
and the combat-callout yellow; that work is finished.

The original implementation remains at
[`ally/src/retired/color_probe.rs`](https://github.com/mxtrnhf/highfleet-allies/blob/main/ally/src/retired/color_probe.rs).

## Retired config keys

Older `ally.json` files may still contain these keys. They are ignored:

- `logging.max_lines` — the line budget is now internal (600 normally, lifted in
  debug mode).
- `logging.log_precombat` — debug mode always enables the pre-combat GUI lists.
- `logging.probe_lines` — debug mode always enables the per-second probes.
- `logging.log_colors` — feature removed.
- `logging.dead_node` — feature removed.
- top-level `debug_overlay` — moved to `logging.debug_overlay`; the old top-level
  form is still read and migrated.

`logging.debug_log` is now the runtime debug switch and defaults to `false`.
The legacy `megalog` cargo feature no longer forces debug mode on.

Non-diagnostic retired keys (`plane_bombers`, `marker_seam`, `marker_delay_ms`,
`max_enemy_ships`) are documented in [retired settings](retired-settings.md).

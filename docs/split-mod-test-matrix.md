# Split-mod compatibility matrix

Use HighFleet Steam 1.163 and Modloader `v4.2.0-preview.1`. Test in a separate
copy of the game with disposable saves. Start with no other mod DLLs, including
QoL. Run each row in a fresh game process, with only the listed DLLs present
and enabled in their matching configs. Use the same campaign save, ships and
control settings across rows.

In the HighFleet development workspace, use `hf_dev_tools/highfleet_headless.py`
to run the startup cases and capture gameplay screenshots in an isolated Proton
session.

| DLLs loaded | Campaign wingmen | Combat AI | Shipworks roster / A.C.E. | Campaign shelling |
|---|:---:|:---:|:---:|:---:|
| None (loader-only control) | — | — | — | — |
| Allies | ✓ | — | — | — |
| AI | — | ✓ | — | — |
| ACES | — | — | ✓ | — |
| CCU | — | — | — | ✓ |
| Allies + AI | ✓ | ✓ | — | — |
| Allies + ACES | ✓ | — | ✓ | — |
| Allies + CCU | ✓ | — | — | ✓ |
| AI + ACES | — | ✓ | ✓ | — |
| AI + CCU | — | ✓ | — | ✓ |
| ACES + CCU | — | — | ✓ | ✓ |
| Allies + AI + ACES | ✓ | ✓ | ✓ | — |
| Allies + AI + CCU | ✓ | ✓ | — | ✓ |
| Allies + ACES + CCU | ✓ | — | ✓ | ✓ |
| AI + ACES + CCU | — | ✓ | ✓ | ✓ |
| Allies + AI + ACES + CCU | ✓ | ✓ | ✓ | ✓ |

## Gameplay checks

- **Allies:** Select a campaign ship as ALLY, deploy it, and switch to and from
  it. Check a carrier's plane selector and launch. Verify reserve count and
  return to the map.
- **AI:** Fight an enemy without a wingman and watch stock pilots use the
  revised steering, tactics and reload. In rows with Allies, include a wingman;
  the player ship should keep manual control. Automatic flares should be off
  for this matrix because QoL is excluded.
- **ACES:** In Shipworks, add friendly and enemy designs, use CLEAR, and run
  FIGHT twice, including with changed rosters. Check that no old enemies carry
  over. Run Silent Strike, Airstrike and conventional and nuclear Missile
  Defense. Check native return to Shipworks, DMG VIEW and Preserve Damage on a
  second launch. Without Allies, expect the stock friendly reserve. With
  Allies, check that the test allies are preselected and the prior campaign
  ally selection is restored when leaving the test.
- **CCU:** Select an eligible 180 mm fleet, pick a city, preview and cancel,
  then fire and abort. Fire again and let the raid finish. Verify the native
  fade and map return, garrison HP on the next encounter, and that an ordinary
  campaign missile defense still works.
- **Absent features:** Visit Shipworks, a campaign map with an eligible fleet,
  and pre-combat; confirm the absent features do not add controls or modify
  stock behavior. A normal combat and stock Shipworks FIGHT should still work.

In ACES + CCU rows, run an ACES Missile Defense after a CCU raid and then run a
second CCU raid. Neither should inherit the other's scenario state or reports.
In rows with Allies and ACES, check both the regular ally panels and the ACES
damage panels. Test `crash_button: true` in a separate disposable session; verify
that it is absent with the shipped `false` setting.

## Hook ownership and load order

- The first enabled one of Allies, ACES or CCU commits ClickFunc
  (`0x14012f120`, 26 bytes) and SomethingInputTask (`0x14018dbf0`, 11 bytes).
  Each later participant registers callbacks with it. Combat AI installs
  neither seam. Callbacks run in ACES → CCU → Allies order regardless of DLL
  load order; the first consumer wins a click.
- In any participating row, expect exactly one `shared GUI: installed` line
  and one `shared GUI: joined` line per additional GUI participant. Check
  all rows for `failed to initialize`, rejected Patchy setup, and overlapping
  patch or source-changed errors. `Found mod` alone is not evidence of init.
- When Allies and AI are both enabled, confirm Allied bomber hooks and AI
  steering and boost are prepared in both load orders. AI resumes at the
  Allies bomber seams at `0x140064a96` and `0x140065750`; neither mod owns the
  other's instructions.
- With all four DLLs, repeat with each of the six relative orders of the
  three GUI participants. Use the actual `Loading mod:` lines to record the
  order; the preview loader enumerates files and does not promise sorting.
  Test AI loaded both before and after the GUI owner.
- Repeat with all DLLs physically present, but disable each in turn (`enabled`
  for Allies and AI; `enable` for ACES and CCU), then disable all four. A
  disabled DLL must never take ownership or register a callback.
- Finally test the all-on row with bundled QoL and the shipped `qol.json`.
  Verify auto-flares, with QoL's flare crash fix enabled. Older DLLs with
  private Patchy engines are outside the loader's shared overlap registry, so
  test any additional legacy mods separately.

For each run, record the game, loader and DLL versions, actual load order,
config deviations, startup result, gameplay result and `Highfleet_*.log` path.
Record startup and gameplay separately: a successful Patchy commit only
establishes that those patches could be installed together.

## Startup results — 2026-11-27

Steam 1.163 with Proton Experimental and Modloader `v4.2.0-preview.1`;
Allies `1.4.0`, AI `1.2.13`, ACES `0.1.0`, CCU `0.1.0`. All 28 startup
cases passed: the 16 subsets above, six GUI load orders, five config-disable
cases, and the all-on run with QoL `0.2.4`. Each enabled feature committed
its hooks, with exactly one GUI owner in each applicable case. AI steering
and boost prepared in both orders relative to Allies; automatic flares
prepared with QoL and were skipped without it.

Startup logs and DLL orders: `/tmp/opencode/allies-split/matrix/results.tsv`.

Standalone ACES gameplay smoke: opened Shipworks, added two Moral copies,
and launched conventional airborne Kh-15 Missile Defense. The encounter
returned to Shipworks with three defenders' native panels; DMG VIEW displayed
the captured panel. Log and screenshots: `/tmp/opencode/allies-split/play-aces/`.

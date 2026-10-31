# Tactical AI carrier launch groups (Steam 1.163)

Status: code and bytes checked against the live Ghidra program on 2026-10-30;
implemented in hf_ai v0.10.3. In-game validation is reserved for the user.
This is separate from strategic `TryLaunchAiAircraftGroup` at `0x1400ef910`.

## Stock tactical flow

`AiShipTick` (`0x140062f90`) checks its shared launch timer at `0x14006591c`:
`COMISS XMM8,[R14+0xcb8]` (XMM8 = 0). When due, it checks clearance against
nearby objects, scans the carrier's plane Link vector `+0x14c8..+0x14d0`, and
selects a ready plane. The scan checks non-null/alive (`+0x48`), repair state
`+0x888 != 1`, activation `+0x890 != 0`, readiness `+0x820 == 1.0`, and
`+0x24c == 0`; `+0x259` and position choose the deck side / outermost plane.
It frees any old logic, calls `Body_AssignAircraftSlots` at `0x140065bab`, then
calls `loadAmmoForAircraft` at `0x140065bd9`. R14 is the carrier and RDI the plane.
Afterward it loads `plane+0xa20`, optionally handles the scripted scenario flag,
and calls `AircraftLogic_SetState(logic,0)` at `0x140065c02`.

Success, no ready plane, and blocked clearance all converge on `0x140065c07`:
`MOV dword ptr [R14+0xcb8],0x40c00000` (6 seconds). At `0x1400658b1` the tick
subtracts the current frame delta from `0x1403c126c`, then stores the timer back.

## Two bounded detours

| Site | Verified bytes | Behavior |
|---|---|---|
| `0x140065bd9` (validation only) | `e8 52 8d fc ff` | Original call to `0x14002e930`, never replaced. |
| `0x140065bde` | `48 8b 8f 20 0a 00 00` | After arming, count the selected plane only if its logic is non-null; replay the original `MOV RCX,[RDI+0xa20]`. |
| `0x140065c07` | `41 c7 86 b8 0c 00 00 00 00 c0 40` | Store the configured group delay plus this frame's delta between group members, otherwise the stock 6 seconds. |

Both callbacks preserve integer/XMM registers and flags, retain call-site stack
alignment, catch Rust panics, and page-check game-memory reads. All site checks
precede patch preparation; both detours install in the existing PatchSession.
No original arming call, takeoff state, plane-selection loop, or clearance test
is replaced. No update hook, sleep, or repeated immediate launch calls are used.

Normal tactical scope uses `stateSelectorAircraft_` (`0x1403bd008`) == 4 and
scripted-scenario byte `0x147fc6fdb` == 0, plus the carrier AI flag `+0xbd4` and
alive state. Selector mapping is independently confirmed in
`AircraftLogic_InitAndPickMode` (`0x14000ae90`): SCR_COMBAT = 4;
bomber scenes = 2/3; aircraft-vs-* = 0/1. See
`hf_notes/docs/findings/plane-combat-modes-and-gunpods.md`.
Player-controlled manual launch `PlayerLaunchPlane` (`0x14007a4c0`) is untouched.

Group state is separate per carrier pointer and node id (`+0x18`). It never retains
dereferenceable game pointers for callbacks and does not read capture-session files.
Clock reversal, stale entries, failed stock attempts, and changed identities
end the partial group. The table is capped at 256 entries. The ordinary stock
six-second pause returns after the configured number of successful launches.

## Configuration and validation

Top-level `plane_group_per_launch`: default 3, clamped to 1..64; 1 installs no
plane-group hooks. Top-level `plane_group_launch_delay_seconds`: default 0.5,
clamped to 0.05..60 game seconds; non-finite values use 0.5. Restart to apply.
These settings are independent of TWR, tactics, flare settings, and missile dumping.

Six host regression tests cover group count, delay/frame compensation, stock
cadence with size 1, failed-attempt cancellation, clock resets/staleness,
independent carriers, and invalid configuration. Windows tests are compile-only;
no DLL or game is executed by the agent.

User test recipe: HighFleet Steam 1.163, hf_ai v0.10.3, the two new defaults.
Start Combat.seria against a carrier with at least three ready aircraft and a
clear deck. Expect launches near t, t+0.5, t+1.0, then the stock six-second wait.
Repeat with allied AI and multiple carriers; with only one/two ready planes;
and with size 1. Change delay to 1.0 and verify spacing in game time. Confirm
strategic air groups and Bombers.seria retain their normal behavior. A blocked
stock clearance check cancels a partial group and can reduce its actual size.
Record logs/video and the exact JSON when reporting in-game results.

## Build and installation record (2026-10-30)

Both source copies cross-built as v0.10.3 for x86_64-pc-windows-msvc.
The release Windows test binaries compiled; they were not executed.
All 74 host math tests passed (6 group, 3 TWR, 19 panic, 35 tactics, 11 curtain)
from the release source in `highfleet-allies/ai`. New modules pass rustfmt;
crate-wide formatting checks report existing formatting differences.

Installed `highfleet-allies/ai` build into the active Steam HighFleet folder as
`Modloader/mods/hf_ai_v0.10.3.dll`, replacing the sole active `hf_ai_v0.10.1.dll`.
Added only the two group keys (3 and 0.5) to the existing JSON; all other values
were retained. Previous DLL and full JSON are backed up under
`Modloader/backups/hf_ai_v0.10.3_20261030T164726Z/`.
DLL SHA256: `43a487cb073df0c487bf873790dab805f96a6d6deb70764e67c94239f00c174e`.
No game or DLL was run by the agent; the user will test gameplay.

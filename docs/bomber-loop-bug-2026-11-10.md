# Root-Cause Report: `ally_bombers` Inner-Loop Hook Bug — 2026-11-10

## Summary

Disabling `ally_bombers` in `ally.json` eliminated the heavy battle AI frame drops:

| Metric (`AI entries = 3`) | `ally_bombers: true` (`21-11-48`) | `ally_bombers: false` (`21-20-52`) |
|---|---:|---:|
| Frame time (`frame_call_us` p50 / p95) | **36.66 ms / 40.30 ms** | **19.54 ms / 20.01 ms** |
| Whole AI task (`ai_us` p50 / p95) | **17.42 ms / 19.81 ms** | **0.56 ms / 0.89 ms** |
| `aiShipDriver` (`ai_driver_us_total` p50 / p95) | **17.37 ms / 19.80 ms** | **0.54 ms / 0.87 ms** |
| Native obstacle scan (`ai_scan_us_total` p50 / p95) | **0.040 ms / 0.105 ms** | **0.023 ms / 0.033 ms** |
| `hf_ally` `VirtualQuery` calls | **45,497,419** | **10,064** |

With 2 allied ships active (`AI entries = 4`), `ally_bombers: true` pushed `ai_us` to **17.96 ms median / 38.29 ms p95** (`max 42.13 ms`).

---

## Exact Root Cause

### 1. `FINAL_ENGINE_SITE` (`0x140065750`) Is the Head of the Per-Part Loop
In `ally/src/bomber.rs`:
- `ENGINE_SITE` is placed at `0x140065734` (once per ship tick, right before the engine-command dispatch loop).
- `hf_ai`'s boost hook sits at `0x140065744` (`MOV RDI, RBX` followed by a 9-byte alignment `NOP` at `0x140065747..0x14006574f`).
- `bomber.rs` placed `FINAL_ENGINE_SITE` at **`0x140065750`** (`MOV RCX, qword ptr [R8 + RDI + 0x8]`), assuming it ran once per ship right after `hf_ai`'s hook.

In reality, `0x140065750` is the **loop target** of `JC 0x140065750` at `0x1400657d3`, which iterates over **every part in `Creature::parts` (`+0x1330..+0x1338`)**:

```asm
140065744: MOV RDI, RBX                          ; RDI = 0 before loop
140065747: NOP word ptr [RAX + RAX*1]            ; 9-byte alignment padding
140065750: MOV RCX, qword ptr [R8 + RDI*1 + 0x8] ; <-- LOOP HEAD (bomber FINAL_ENGINE_SITE)
...
1400657b8: ADD RDI, 0x18                         ; advance to next part in Creature::parts
...
1400657d3: JC  0x140065750                       ; jumps back into FINAL_ENGINE_SITE for every part!
```

For a 464-part allied ship, `on_final_engines` (`restore_engines`) is called **464 times per AI tick per ally** instead of once.

### 2. Unconditional `flight(ship)` and `VirtualQuery` Calls Before Checking `RUNS`
Inside `restore_engines(ship, command)` (and `raise_dive_speed`, `boost_engines`, `release`, `blocks_launch`):
- `flight(ship)` is called **before** checking whether `ship` even has an active bombing run in `RUNS`.
- Even when an ally carries **zero bombs** (`RUNS` is empty), every iteration of the 464-part loop calls `flight(ship)`, which executes page-checked `diag::rd` (`VirtualQuery`) reads across the ship, its target, and `root_part(ship)` (walking the parent chain).
- When `RUNS` *does* have an active entry, `restore_engines` calls `flight(ship)` and then calls `boost_engines(ship, command)`, which calls `flight(ship)` a **second** time on every part iteration.

---

## Why This Explains All Three In-Game Observations

1. **"Performance slightly improves after enemies take off":**
   - In `bomber.rs`, `flight(ship)` checks `landed_enemy(target)`, which requires `target + OFF_MODE == 3` (landed on the ground).
   - While the enemy is still landed on the ground (`mode == 3`), `flight(ship)` passes `landed_enemy(target)` and executes the rest of `flight(ship)` (`root_part(ship)` parent walk, velocity, radius, mass, thrust, and `boost_available` temperature/lock reads) **464 times per frame per ally**.
   - As soon as the enemy takes off (`mode != 3`), `landed_enemy(target)` returns `false` early, cutting the number of `VirtualQuery` calls per part iteration roughly in half.

2. **"Performance drops disappear after all enemies die":**
   - Once all enemies die, allied ships leave combat state (`state == 1`) or the battle ends, so `AiShipTick` (and the 464-iteration loop at `0x140065750`) stops running.

3. **"Disabling `ally_bombers` made the game run much better":**
   - With `ally_bombers: false`, the 5 bomber hooks are not installed: `VirtualQuery` calls drop from **45.5M to 10k**, and 3-ship AI time drops from **17.42 ms to 0.56 ms**.

---

## Proposed Fix (Applied in hf_ally v1.0.5)

1. **Skip non-zero loop iterations at `FINAL_ENGINE_SITE` (`0x140065750`):**
   - At `0x140065750`, `RDI` is `0` on the first iteration (right after `0x140065744: MOV RDI, RBX`) and non-zero (`0x18, 0x30, ...`) on all subsequent iterations (`0x1400657b8: ADD RDI, 0x18`).
   - Use `Trampoline` conditional jump (`test rdi, rdi; jnz skip`) so iterations `1..N` execute the original `MOV RCX, [R8 + RDI + 8]` directly and never enter the Rust callback.

2. **Check `RUNS` before calling `flight(ship)` in secondary hooks:**
   - In `raise_dive_speed`, `boost_engines`, `restore_engines`, `release`, and `blocks_launch`, check whether `RUNS` has an active course for `ship` before calling `flight(ship)`, and pass the already-validated `&Flight` from `restore_engines` into `boost_engines` instead of calling `flight(ship)` twice.
   - In `steer(ship)`, check `flight(ship)` (which is cheap and checks `live`, `ai_flag`, `mode == 1`, `state == 1`, and `landed_enemy`) before walking the bomb vector in `first_bomb(ship)`.

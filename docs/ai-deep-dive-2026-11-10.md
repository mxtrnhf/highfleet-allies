# AI Deep-Dive Findings (Ghidra 1.163) — 2026-11-10

This document consolidates the three read-only Ghidra reverse-engineering reports (`aircraft-and-parity.md`, `obstacle-scan.md`, `threat-and-tick.md`) investigating why `SomethingAITask` (`0x140011180`) spikes to **~40.3 ms** when 4 AI ships are active, even with `ally_panels: false`.

---

## 1. Where the 40 ms Spike Lives (and What Is Exonerated)

From the `hf_perf v0.6.1` capture (`hf_perf_v0.6.1_1794330562959474400_312.csv`, 101 unpaused rows with 4 AI-list entries, `ally_panels: false`):

1. **`SomethingAITask` (`0x140011180`) only runs two workloads inside its timer (`ai_us`):**
   - The per-entry `aiShipDriver` (`0x1400739c0`) loop over `0x143faa608..0x143faa610` (`CALL 0x1400739c0` at `0x14001123a`).
   - One unconditional call to `AircraftFormationManager` (`0x140009250`) at `0x140011286`.

2. **Why odd/even frames alternated between `40.274 ms` and `0.144 ms`:**
   - `hf_ai` v1.1.0 had `"ally_tick_skip": true`, which hooks `0x14001123a` and skips `aiShipDriver` on odd frames **only** for player-side ships (`kind == 1 && side > 0.0`).
   - On odd frames, the 2 enemy `aiShipDriver` calls **plus** `AircraftFormationManager` **plus** all 4 virtual active gates (`[vtable+0x78]`) still ran and took only **0.144 ms total** (`~0.06 ms` per enemy ship).
   - On even frames, adding the 2 player-side (allied) `aiShipDriver` calls jumped `ai_us` from **0.144 ms to 40.274 ms** — i.e. **~19.95 ms per allied `aiShipDriver` call**.

3. **Why `hf_ai`'s internal span columns only showed ~0.2 ms:**
   - Three of `hf_ai`'s five spans (`ai_creature_update`, `ai_threat_scan`, `ai_tracking`) hook `Creature_Update` (`0x14007eb50`), which is a vtable method called **outside** `SomethingAITask` (5 calls/frame on both even and odd frames).
   - Only `ai_steering` (`0x140064a8d`, ~18 µs/ship) and `ai_boost` (`0x140065744`, ~1.5 µs/ship) sit inside `AiShipTick`, and they only time `hf_ai`'s own Rust callbacks, not the surrounding native function.

---

## 2. Exact Map of `aiShipDriver` (`0x1400739c0`) and `AiShipTick` (`0x140062f90`)

### 2.1 `aiShipDriver` (`0x1400739c0`)
- **Kind 2 (`+0xbd8 == 2`, ground defence):** Returns `1` immediately (`0x1400739e6`).
- **Kind 3 (`+0xbd8 == 3`, launched missile/aircraft body):** Checks engine count (`+0x1330`) and loaded weapon (`+0x1360`); if unflyable/unarmed, calls `[vtable+0x48]` (`0x140073b6e`, the only mid-AI-task creature kill) and returns; else calls `AiShip_ChooseTarget`, `Creature_AiAutoExtinguish`, and `AiWeaponController`.
- **Kind 1 (`+0xbd8 == 1`, ship):**
  1. Clears `creature+0xca0` and calls **`AiShip_BuildThreatList(this, &+0xca0, 0.0, 0)`** (`0x140073bcb`) unconditionally on every tick.
  2. Calls `ShipStatusCheck` (`0x14006ab70`).
  3. For states `0, 1, 2, 3, 6`: calls **`AiShip_ChooseTarget`** (`0x140078b40`, which is gated by a 3.0 s timer at `+0xb28` on 149/150 frames) and the **per-tick missile/threat chooser `0x14006c100`** (`0x140073c7b`), then updates `SetLink(+0xb10)`.
  4. Dispatches on state (`+0xbdc`):
     - States `1, 2` (combat / bombing) → **`AiShipTick` (`0x140062f90`)** (`0x140074017`).
     - States `0, 3, 4, 7` → **`AiShip_SteerController` (`0x14006cb30`)**.

### 2.2 Candidate Hotspots Inside `aiShipDriver` / `AiShipTick`

#### A. Obstacle Scanning & List Building (`AiShip_ScanObstacles` `0x14024cc20` + `AiShip_AddObstacleNode` `0x14024ce50`)
- **Where called:** `0x140064e6f` in `AiShipTick` (after freeing the previous tick's list at `0x140064d60–0x140064dc4`), and `0x14006cf83` in `AiShip_SteerController`.
- **What it walks:** Walks the global flat body array `0x143a33c90` (count at `0x143a51150`, max 15,000, rebuilt once per frame in `SomethingPreTask` by `Creature_RecalcStatsPass` `0x140287370`).
- **Filters:** Active state (`vtable+0x78`), not own root, not own ship (`body+0xb8 != creature+0xb8`), body class (`(flags & 0xf) == 0xf`), non-null `b2Body` (`+0x178`), and mass ≥ `0.1 * ownMass` (unless static, explosive `>= 500`, or bombing mode `+0xd70 == 2` with part flag bit 21). **Friendly ships are NOT filtered out.**
- **Compound recursion & `Θ(n²)` insertion:**
  - `range = clamp(4.0 * |v|, 200.0, 800.0)`.
  - If a ship's compound frame (`(flags & 0x1f) == 0x1f`) is within `range`, `AiShip_AddObstacleNode` recurses into **every child part** in `frame+0x100`.
  - For each part within `range`: computes world bbox center (`vtable+0xe8`), quantized distance (`SqrtCache[(u32)(d² * 10.0)]` at `0x143a118a8` when `d² < 10000`, else `sqrtf`), bearing `atan2f(-dx/d, -dy/d)`, shadow half-angle `atan2f(r, d)`, walks the nearer nodes in `creature+0xd78` (`gap < new_gap`) for shadow occlusion, allocates two heap blocks (`New(0x38)` + `New(0x18)`), and then **unconditionally walks all farther nodes to the tail**, freeing any node whose bearing falls inside the new node's `[lo, hi]` shadow cone.
- **Why allies are Prime Suspect #1 here:** Allies fly inside the player's formation at speed (`range` up to 800), placing the player flagship and second ally (400–900+ parts) inside their obstacle scan radius every frame, whereas distant enemies only test top-level compound bounding boxes and don't expand child parts.

#### B. Threat List Rebuild (`AiShip_BuildThreatList` `0x1400625a0`)
- Runs every tick for every ship. Prunes launched-missile lists (`+0xa48`), builds a temporary red-black tree (`New(0x30)` per node), and when no enemy launched missiles exist, walks the 15,000-slot flat body array looking for aircraft (`logic == 500`) or strategic launches (`DAT_147fc6fda != 0`).

#### C. Non-Obstacle Loops in `AiShipTick` (`0x140062f90`) and `AiWeaponController` (`0x14007a8d0`)
- **4 global creature scans** (`L1` bombing probe, `L2` separation, `L3` keep-away, `L4` group centroid).
- **`Ballistics_PredictHit` (`0x1401b5420`):** Simulates 500 trajectory steps × `K` target-side creature circles (with a `sqrt` per step per circle). Also called by `hf_ally::bomber::predict_bomb` at 10 Hz during bombing dives.
- **Friendly-fire check in `AiWeaponController` (`AnyFriendlyCreatureWithinRadiusMul` `0x14021c790`):** Scans all creatures **once per gun** with identical arguments `(shooter, target_pos, side, 2.0)` — hoistable to once per target.
- **Role-2 bombing `AiShip_ChooseTarget` bug (`0x140078edc`):** Contains a `sprintf("CHECK ERROR: %s, %d")` + log write inside a per-creature loop when a role-2 ship's target isn't in the allowance tree.
- **`hf_ally::bomber` hooks inside `AiShipTick`:** `hf_ally` hooks 4 sites inside `AiShipTick` (`0x140064a96`, `0x140064f58`, `0x140065734`, `0x140065750`) and 1 in `AiWeaponController` (`0x14007bf15`). Each hook calls `flight(ship)` and `first_bomb(ship)` for tracked allies, doing page-checked `diag::rd` (`VirtualQuery`) reads across the bomb vector (`+0x1438`) and root-part parent chain. This explains why `hf_ally` still made **3.945M `VirtualQuery` calls** in the `ally_panels: false` run!

---

## 3. Immediate Verification & Optimization Plan

1. **Decisive 1-Run Partition (`hf_perf v0.6.2` + `ally_tick_skip: false`, already installed):**
   - `hf_perf v0.6.2` wraps `aiShipDriver` (`0x14001123a`) and both `AiShip_ScanObstacles` sites (`0x140064e6f`, `0x14006cf83`) with unwind-safe `CALL` relays.
   - Comparing `ai_driver_us_total` and `ai_scan_us_total` in the CSV immediately tells us what fraction of the ~20 ms/ally spike is inside `AiShip_ScanObstacles` vs. outside it (`AiShipTick` / `hf_ally::bomber` / `BuildThreatList`).

2. **Stage 1 Native Accelerations (Ready to Implement Once Partitioned):**
   - **If `ai_scan_us_total` dominates:** Build a shared per-AI-step candidate snapshot + contiguous arena obstacle builder (`obstacle-scan.md` §6, preserving the exact 11-point bit-identical contract: `SqrtCache` for `d² < 10000`, `atan2f(-ux, -uy)`, 3× round-half-away angle wrap, strict `gap <` walk, inclusive non-wrapped vs. exclusive wrapped interval check, and materializing only the final surviving nodes into `creature+0xd78`).
   - **If non-scan `ai_driver_us_total` dominates (or alongside it):**
     - Eliminate per-field `VirtualQuery` calls inside `hf_ally::bomber`'s 5 per-tick `AiShipTick` callbacks (which currently run `first_bomb` + `flight` 4–5 times per ally per tick).
     - Hoist `AnyFriendlyCreatureWithinRadiusMul` out of the per-gun loop in `AiWeaponController` and share a per-frame `AiFrameCreature` / `AiFrameBody` index across `SomethingAITask`.

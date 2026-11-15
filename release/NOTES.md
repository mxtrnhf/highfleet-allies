# HighFleet Allies v@PACK_VERSION@

## What changed

### AI dive avoidance & collision safety (`hf_ai`)

- **Cooperative dodge boosts (won't fight obstacle avoidance or gravity hold)**
  - Previously, tactical dodge boosts blindly overwrote the engine thrust angle, fighting the game's native obstacle avoidance and gravity compensation—causing ships to slam into terrain or other hulls during a dash. Now angle overrides only apply when native thrust agrees within 60 degrees; otherwise it scales along the native vector or yields.
- **Hull-size-aware dive distance & multi-ship keep-out barriers**
  - Previously, dive range and spacing used ship centers, so fast interceptors misjudged large ships, sidestepped into nearby hulls, or yielded way too early in fast dogfights. Now `dive_range` and keep-out barriers (`120–280 u` surface-to-surface) use actual hull radii against the target and nearby ships, and emergency braking yield only triggers on imminent impact (`< 70 u` gap or `< 0.45 s` TTC).
- **Hostile-only dive denial**
  - Previously, a friendly ship passing underneath could still trigger dive denial. Now only airborne enemy ships trigger it.
- **Marginal-TWR ships (e.g. Wasp) still sidestep**
  - Previously, ships just above `twr_cutoff` (`2.0`) got almost zero dodge authority, so `AI DIVE` barely moved them and immediately got overwritten by `AI AIM` or `AI STOCK`. Now vertical altitude-matching still ramps from `0%` at `twr_cutoff` to `100%` at `TWR 4.0` so low-TWR ships keep lift, while lateral sidesteps keep a `40%` floor so they actually dodge sideways.

### Ally retreat (`hf_ally` & `hf_ai`)

- **Allies can now retreat off any arena border (left, right, or top)**
  - Previously, retreating allies only extracted via the top/retreat point, making flank retreats slow and awkward without touching the player's retreat point. Now retreating AI allies extract as soon as they reach any arena border (`xmin`, `xmax`, or `ymin`).
- **Dedicated GTFO steering with lane awareness and cruise boost**
  - Previously, retreating allies kept running combat `Aim`/`DiveDodge` tactics, got held back by the `300 u` edge margin, and could get stuck around enemies. Now they pick the nearest unblocked border (avoiding borders with an enemy in the exit lane), steer around nearby hulls, ignore the edge margin, and use a gentle `1.25x` cruise boost instead of burning engines at `2.0x`.

### Co-op compatibility (`hf_ally`)

- **Second-player co-op compatibility (issue #1)**
  - Previously, second-player co-op setups could conflict with ally handling. Added compatibility support in `hf_ally`.

### Notes

- `Allies_Manual.pdf` is still from an earlier version and does not yet reflect the new hull-radius dive behavior or ally retreat changes. Please test in game and share feedback!

Includes **hf_ally v@ALLY_VERSION@**, **hf_ai v@AI_VERSION@** and bundled QoL.

## Install

For HighFleet **Steam 1.163** with Modloader v4. Back up `Saves`, extract the ZIP
into the game directory, and remove older ally/AI DLLs so only one of each remains.
Use only one QoL build: do not stack bundled `highfleet_qol.dll` with another QoL
fork. Merge the included configs with your custom settings.

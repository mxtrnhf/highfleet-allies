# Handover: `Allies_Manual.pdf` (`hf_ai` & Dive Params Update)

Reviewed against `highfleet-allies/ai` (`v1.2.6`) and `docs/Allies_Manual.pdf` (pages 11–13).

## 1. Stale / Retired Keys in the Manual's `hf_ai.json` Snippets

- **Page 13 (`tactics` screenshot):** `aim_fire_probability` and `aim_fire_range` were **retired in `v0.10.19`** (offensive early fire was removed). The parser ignores them, so they do nothing now. Remove them from the screenshot and mention that they are obsolete.
- **Page 11 (`hf_ai.json` top block):** Missing `twr_limit` (default `5.0`) and `twr_deceleration` (default `0.75`), added in `v1.2.0`/`v1.2.1` to scale `hf_ai` boosts for ultra-high-TWR ships without touching stock thrust/avoidance.

## 2. Dive & Maneuver Semantics That Changed (`v1.2.2` – `v1.2.6`)

Update the bullet points on **pages 11 and 13** so players/modders know what the parameters actually do now:

- **`twr_cutoff` (page 11):**
  - Still a hard minimum Shipworks TWR gate (`2.0` default).
  - Above `twr_cutoff`, maneuver authority now ramps smoothly up to `TWR 4.0`:
    - `dive_level_weight` (pulling onto the target's altitude) scales from `0%` at `twr_cutoff` to `100%` at `TWR 4.0`, so marginal-TWR ships (e.g. Wasp) stay level and keep their lift.
    - Lateral sidestepping and boost keep a `40%` floor (`4.5` blend weight floor) so marginal-TWR ships still dodge sideways.
- **`dive_range` (page 13):**
  - **No longer center-to-center.** It is now **surface-to-surface hull gap** (`distance - own_radius - target_radius` using `Creature::bounding_radius`).
  - Against a large brick (`r ~ 450u`) with a Lightning (`r ~ 80u`), `dive_range: 600` triggers at `~1130u` center-to-center instead of `600u`.
- **`dive_cone_deg` (page 13):**
  - The manual says "radius of 45 degree cone". Worth noting `dive_cone_deg` is a configurable half-angle (default `45`, `15%` hysteresis once aligned).
- **Collision & Native Avoidance Cooperation (new behavior to document):**
  - **Keep-out barrier:** Within `120..280u` surface-to-surface of the target or up to 3 nearby ships (friendly or hostile), tactical waypoints bend tangentially/outward so ships don't sidestep into hulls.
  - **Imminent collision yield:** If closing inside `< 70u` surface gap (or `< 0.45 s` TTC within braking distance), `hf_ai` yields to stock `AiShipTick` braking.
  - **Cooperative boost:** `dive_boost` / `aim_boost` angle overrides (`write_escape`) only fire when the native thrust vector (which includes obstacle avoidance and gravity hold) agrees within `60 deg`; otherwise it scales thrust along the native vector or yields when opposed.
- **`dive_dwell_s` & `dive_chance` (page 13):**
  - Manual screenshot shows `dive_chance: 0.5`, `dive_dwell_s: 0.1`, `dive_cooldown_s: 1.0` (custom preset), whereas compiled/player defaults are `0.8`, `0.3`, `2.0`. Worth noting which values are compiled defaults vs. the example preset—especially since `0.5` causes `AI DIVE SKIP` on half of all alignments in the debug overlay.

## 3. Small Notes / Future Follow-ups

- **Page 13 (`aim_tolerance`):** *"Uses ship center, not ship total size. I should really patch that, eh?"* — `aim_ray_hits` in `tactics_math.rs` still checks `cross.abs() <= p.aim_tolerance` without adding `ship_radius`. Easy future fix if you want `aim_tolerance` to scale with big bricks too.
- **Page 2 (`debug_overlay`):** Might be worth adding a one-liner explaining `AI DIVE SKIP` (target stayed in cone for `dive_dwell_s` and `dive_chance` rolled "no", cooling down for `dive_cooldown_s`).

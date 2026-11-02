# Combat ship switching

Requires `spawn_ally: true` and `ship_switching: true` (default).

- **1**: take the ship actually displayed in the top ally panel.
- **2**: take the ship actually displayed in the bottom ally panel.
- **TAB**: take the next deployed, living friendly ship in native roster order,
  wrapping around. This includes your original ship and allies beyond the two panels,
  but not ships waiting in reserve.

The selected ship becomes player-controlled immediately at the next combat tick.
Your former ship becomes an ordinary AI ally, replacing the selected ship in ally
tracking and panel order. The panel textures refresh on the next native render pass;
old textures are invalidated rather than briefly showing the wrong ship.
Held gun/flare/boost inputs move to the new player ship; pending bomb commands do not.
Retreating, destroyed, inactive and non-flying ships cannot be selected. An empty or
hidden panel does nothing. Keys act once per press, not repeatedly while held, and
are ignored during pre-combat, paused time, dialogs or when another application has focus.
If several switch keys are pressed together, 1 takes precedence, then 2, then TAB.

The old player ship uses the game's native AI. Installed/enabled `hf_ai` picks it up
through the existing AI-flag gates (`ai/src/tactics_math.rs`, `ai/src/ai_flares.rs`);
its allied-AI settings still apply. No second AI instance or DLL reload is needed.
F8 and low-fuel retreat now apply to the old player ship, not the newly controlled ship.
Set `ship_switching: false` if custom game keybindings or another mod use these keys;
this feature does not rewrite or swallow native keyboard bindings.

## Native evidence and safety (Steam 1.163, checked 2026-11-03)

Implementation: `ally/src/ship_switch.rs`. Existing RE recipe:
`hf_notes/docs/findings/battle-ally.md`, sections “The vanilla reserve, wingman and
redeploy machinery” and “Checklist”. Live Ghidra verification used only the 1.163 program.

- `Profile_SetControlledShip` **0x1402a7e30** sets the old ship's `+0xbd4` AI flag,
  clears the new ship's flag, updates the profile's native link and refreshes battle
  backgrounds. The handoff also calls native `SetLink` **0x140194de0** on control slot 0
  at **0x143a1c260**, changes the old ORDER `+0xd70` to 0 and the new ORDER to 1.
  All these native entry points retain the mod's existing prologue/version checks.
- Native `SomethingAITask` walks the AI vector **0x143faa608..0x143faa610**
  (bounded `Link` entries, target at +8). The player ship is **not** registered
  there in battle, which is why v0.14.5 refused every switch in the first test.
  The fix adds the old player exactly as `Creature_RegisterInWorldLists`
  (0x14006bf60) does: native `Link_ConstructFromBody` 0x140194bd0, then native
  vector copy/grow (`Link_CopyConstructor` 0x140194b80 /
  `LinkVec_EmplaceReallocate` 0x140004b00), then `Link_UnlinkFromNode`
  0x140194d20. The promoted ally is removed from that vector with the same native
  Link operations, so the AI task cannot drive the ship the player now flies.
  Every native helper is byte-checked at install; failure leaves the battle hook
  uninstalled rather than switching control without AI registration.
- `Input_KeyDown` **0x1401896a0** and `Input_KeyUp` **0x14018a530** confirm boost
  `+0xba8`, gun groups `+0xbbc/+0xbbd`, bomb release `+0xbbe`, flare controls
  `+0xbbf..+0xbc1` and bomb-held byte `+0xce8`. The former player is reset to
  neutral inputs so held player commands cannot stick on its AI ship.
- Native combat-input guards use **0x147fc6fb7** and frame dt **0x1403c126c**;
  active input dialogs are at **0x143a13e08**.
- `SetDefaultKeyBindings` **0x14018d3d0** has no 1/2/TAB combat binding.
  `Input_HandleKeyDown` **0x140187100** does toggle **0x1439e92c8** for TAB, but
  the only xrefs found are that handler's own read/write: no consumers were found.
  It is not a known active combat action; customized bindings can still conflict.
- The pinned highfleet-rs revision and current upstream `src/v1_163.rs` were
  checked: neither provides a Creature layout. No duplicate owning structs or
  dependency updates were added. Shared Link offsets come from the pinned library.
- Panel key selection uses the last **composited ship identity**, not a freshly
  recomputed ally index. Cached identities must still be current roster members
  before any dereference. Handoffs exchange ally-list entries in place, preserving
  the number of deployed AI allies and existing retreat/deployment accounting.
- Before the original player becomes AI, its ammo funding is recorded once for
  the existing ammo-doctrine/refund safeguard; repeated handoffs do not overwrite it.

## First test run and fix (2026-11-03)

The first installed build loaded and enabled the feature, two allies deployed, and
the panels drew, but every attempt logged
`ALLY switch ignored: invalid profile or previous ship ... absent from native AI list`.
Live analysis of `Input_HandleKeyDown`, `Creature_RegisterInWorldLists`
(0x14006bf60) and `SomethingAITask` (0x140011180) showed the player ship was never
added to the AI vector at spawn: registration-time AI-flag membership is real and
the old notes' guess that the player stays registered was wrong. The fixed build
rebalances that vector with the native Link helpers described above. The success
line now ends with `(AI list N ships)`; a specific warning replaces the old combined
diagnostic if native registration still fails.

## In-game validation still required

Release cross-build and Windows-target Clippy passed with default features and
with `megalog`; existing warnings remain. The new module passes rustfmt checking.
Crate-wide `cargo fmt --check` still reports formatting differences across existing
files; no unrelated formatting cleanup was applied. No automated tests were added.

Not yet tested in game. On Steam 1.163, try 1/2 repeatedly while firing, boosting
and moving; TAB through three or more deployed allies; then switch back to the
original ship. Verify panel identities, player steering, AI fire/evasion, F8,
low-fuel retreat and no extra ally spawn. Check an empty bottom panel, a destroyed
or retreating ally, pause/menu/Alt-Tab, battle retry, victory/defeat and ammo refunds.
Repeat without `hf_ai`, with it enabled, and with `ally_panels: false` (TAB only).
Logs record `ALLY control switched` or a refused native-AI registration check.

# Cannon-shell launch-speed randomization — Steam 1.163

Verified against the live Ghidra program `Highfleet.exe.unpacked.exe v1.163`
on 2026-11-09. Static code evidence; no runtime speed measurement performed.

## Function and mechanism

**`Body::BulletImpact__`, entry `0x140052a80`** (Ghidra symbol/comment also rendered
as `BulletImpact??`) initializes each cannon shell. It is called by
**`FireShell`, `0x1400300d0`**, after constructing the projectile.

The launch sequence is:

```text
shell.velocity = sourceBody.velocity                 // if sourceBody != null
speed_roll = HGE->Random_Float(0.98f, 1.2f)
launch_speed = speed_roll * ammo.speed * speedFactor
spread_angle = HGE->Random_Float(-spread, +spread)     // a SEPARATE roll
impulse = rotate(fireDir, spread_angle) * launch_speed * shell.mass
shell.ApplyLinearImpulse(impulse)
```

Thus **98–120% applies to the muzzle-speed contribution**, not to the final
world-space speed including inherited source-body velocity. The impulse's mass
factor cancels the inverse mass when changing velocity. `FireShell` supplies
`speedFactor = 1.0` for its ordinary cannon cases. It starts the shell at the
gun's native world origin plus `10 * fireDir` and finds `sourceBody` by walking
the firing gun's Body master chain; that need not be the creature's root body.

This is not merely random angular spread or a random visual tracer effect.
The speed roll feeds the physical launch impulse. The midpoint multiplier is
`(0.98 + 1.2) / 2 = 1.09`; this is an estimate for an overlay, not the actual
next round's unknown roll or a guarantee that an individual shot will hit.

## Instruction-level evidence

Within `Body::BulletImpact__`:

| Address | Instruction / meaning |
|---|---|
| `0x140052bc7..0x140052bda` | Read `sourceBody +0x130/+0x134`; call `Body_SetLinearVelocity` (`0x140051e20`) on the shell. |
| `0x140052bdf` | Load HGE interface from global `0x143a119e0`. |
| `0x140052be6` | Load XMM2 from `0x140354758`: bytes `9a 99 99 3f` = **1.2f**. |
| `0x140052bee` | Load XMM1 from `0x140354740`: bytes `48 e1 7a 3f` = **0.98f**. |
| `0x140052bf9` | `CALL [RAX + 0x110]`: HGE `Random_Float(min, max)`. |
| `0x140052c07` | Copy returned XMM0 into XMM9. |
| `0x140052c12` | `MULSS XMM9, [RDI + 0x158]`: multiply by `Ammo::speed`. |
| `0x140052c36` | Multiply XMM9 by `speedFactor` from the argument stack. |
| `0x140052c20..0x140052c40` | Construct `-spread/+spread` and call the same random-float method again, independently of speed. |
| `0x140052c59` | `MULSS XMM9, [RBX + 0x1d8]`: multiply by projectile mass. |
| following `0x140052c59` | Rotate direction by the spread angle, scale by XMM9, and dispatch the shell's linear-impulse method through its vtable. |

The first four floating/integer arguments follow the Windows x64 ABI:
RCX = HGE `this`, XMM1 = minimum, XMM2 = maximum; result = XMM0.

## Scope and limitations

`BulletImpact__` is used by the cannon dispatch cases (caliber identifiers
5, 10, 15, 20, 25, 30, 40, and the special 90 case). Rockets and canister use
other branches in `FireShell`; do not infer their launch speeds from this rule.
After launch, gravity, damping and (for AP) quadratic drag can change speed.
Guidance, collisions, recoil and target acceleration also prevent a deterministic
aim marker from promising a hit.

Earlier supporting notes: `hf_notes/docs/findings/combat/firing-and-shells.md`,
section “How a cannon shell is set up”, launch-impulse bullet. That note already
records the same `Random_Float(0.98, 1.2)` mechanism; this file adds exact 1.163
instruction addresses and distinguishes inherited velocity from muzzle speed.

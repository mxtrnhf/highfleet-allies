# Build and release HighFleet Allies

## Build the player pack DLLs

Use stable Rust. Run this command separately from each crate directory:
[`ally/`](../ally), [`ai/`](../ai), and [`qol/`](../qol).

```sh
cargo xwin build --release --locked --no-default-features --target x86_64-pc-windows-msvc
```

The command uses `cargo-xwin` to build Windows DLLs from Linux. On Windows, use
`cargo build` with the same options instead. A successful build doesn't validate
hooks or gameplay; verify those in Steam 1.163 before claiming in-game support.

The experimental `hf_perf` and `hf_native` DLLs and their research notes live
on the `perf/native` branch. They are not built or shipped with the player pack.

## Package and publish

Run [Build and publish release](../.github/workflows/release.yml) in GitHub Actions
with the three-part pack version, without the `v` prefix. The workflow builds all
three DLLs on Windows, packages the tracked presets, fills version placeholders
in `release/README.txt` and `release/NOTES.md`, and publishes the ZIP to GitHub
Releases. Review the release notes and the workflow's gameplay validation text
for the chosen build before publishing.

For local packaging, build all three DLLs and use a clean tracked working tree.
From the repository root, run the following with your chosen pack version:

```sh
python3 tools/package_release.py --version X.Y.Z --output ../release_packs
```

The pack tag is separate from the crate versions. The ZIP's `manifest.json`
records the exact component versions, source commit, validation notes, and
per-file SHA-256 hashes. See [player defaults](player-defaults.md) for how the
tracked presets and compiled defaults are maintained.

## Ally source ownership

`ally/src/lib.rs` is the DLL entry point and wires the modules together; it isn't
a shared state container. `config` owns settings, `precombat` owns pre-combat
buttons and GUI-list validation, and `battle` owns battle arming, tracked allies,
deployment, and retreat. `ally_ammo` owns doctrine, funding records, and refunds.
`sound` retains native sound-status storage, while `battle_diagnostics` owns
callback-local probes and tracking. `native` holds shared addresses and scalar
memory helpers; `native_layout` defines layout contracts. `bootstrap` coordinates
feature patches through one `PatchSession` before permanent installation.

Keep state with the operations that enforce its lifetime. A cached address is
not proof of a live object. Preserve membership checks, traversal bounds, game
ownership, callback order, and blocking versus non-blocking lock behavior when
moving code. See [hf_notes](https://github.com/mxtrnhf/hf_notes) for
reverse-engineering findings on Steam 1.163.

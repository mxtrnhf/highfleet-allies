# hf_native v0.1.0 — experimental native-engine acceleration

Independent Modloader v4 DLL for **HighFleet Steam 1.163**. Source lives on
`highfleet-allies` main. This is a tester build, not a measured FPS claim or a
component of the ordinary player release pack.

**No panels/read-model changes, AI tick skipping, slower decision cadence,
physics reductions, or consumerless worker snapshots.** Existing hf_ally,
hf_ai and QoL stay separate and are not rebuilt by this crate.

## Build

```bash
cargo fmt --check
cargo xwin build --release --locked --target x86_64-pc-windows-msvc
cargo xwin clippy --release --locked --target x86_64-pc-windows-msvc -- -D warnings
```

Output: `target/x86_64-pc-windows-msvc/release/hf_native.dll`. Distribute/install
with its versioned name **`hf_native_v0.1.0.dll`**. No game execution is part of
the build. No new regression tests were added (repository permission policy).

## Install / disable

1. Exit the game. Back up saves before testing native hooks.
2. Copy the versioned DLL to `Modloader/mods/`; keep only one active hf_native DLL.
3. Copy `config/hf_native.json` to `Modloader/config/hf_native.json`.
4. Set `enabled` to `true`, then launch through Steam normally. Missing configs
   are generated with `enabled: false`; native changes are deliberately opt-in.
5. Restart after changing feature switches. Do not unload or inject during play.

To return to native behavior, set `enabled: false` and restart, or move the DLL
outside `Modloader/mods/` while the game is stopped. This does not disable
features in other DLLs, including an older perf-branch hf_ai's ally tick skip.

## Configuration

| Key | Default | Meaning |
|---|---|---|
| `enabled` | false | Master startup installation switch |
| `link_search` | true | Native reference-removal acceleration and aggregate counters |
| `tail_probe` | true | Newest-cell probe for `Link_UnlinkFromNode` and the `SLink` destructor (measured 58.5% hit rate) |
| `tail_probe_remove` | false | Newest-cell probe for `RemoveNodeLink`; measured 1.7% hit rate at 453.6 hops/call, so off by default |
| `parts_direct_append` | true | Direct in-place Link construction in `Body_CollectSubtreeLinks` when capacity is available (elides temporary Link register/unlink/free churn) |
| `contact_pool` | true | Scoped native contact allocation reuse |
| `contact_pool_capacity` | 4096 | Maximum cached free blocks per contact size class; 1..65536 |
| `diagnostics_interval_seconds` | 10 | Periodic atomic-counter snapshots in Modloader logs; 0 disables |

Logs identify the effective feature choices and byte-guard failures. Installation
fails before permanent patching when requested modules cannot validate their
supported native code. A log saying the DLL was found is not proof it installed.
The diagnostic thread reads counters only, never native game objects.

## Test instructions

Use the same saved encounter, friendly/enemy heavy cruisers, configuration and
camera in each run. Keep the other DLL versions/settings fixed.

1. Restart with `enabled: true`. Confirm an `hf_native ... installed` line.
2. Run the heavy battle for at least 30 seconds. Exercise destruction, retreat,
   ship switching and RETRY; verify native behavior and unchanged panel visuals.
3. Restart with `enabled: false`, repeat the same encounter, then compare.
4. Report crashes, visible/gameplay differences and the newest Modloader log.
   The aggregate counters explain exercised paths, not FPS or frame-time gains.

For optional Linux CPU sampling during the battle:

```bash
PID=$(pgrep -n -x Highfleet.exe)
perf record -e cpu-clock:u -F 999 --call-graph dwarf,8192 \
  -p "$PID" -o /tmp/opencode/hf-native-on.perf.data -- sleep 10
perf script -G -i /tmp/opencode/hf-native-on.perf.data \
  -F comm,tid,time,period,ip,sym,dso > /tmp/opencode/hf-native-on.leaves.txt
```

Select the actual game PID, not a Steam/Proton launcher. Use `hf-native-off`
filenames for the disabled comparison to preserve both captures. Windows stacks
may not unwind through Wine; leaf IPs remain useful. Do not infer savings from
one sampled function disappearing or from different battles.

The older hf_perf 0.6.0 and perf-branch hf_ai tick-skip hooks compete at
0x14001123a. This DLL must not work around that conflict by bypassing guards;
the native tester capture does not require hf_perf to initialize successfully.

## Evidence and credits

See [the native architecture review](../docs/native-engine-dll-architecture.md)
for verified addresses, side effects, lifecycle constraints and unresolved
measurement/proof gates. Cross-building does not validate runtime hooks.

Built on logdot's [Highfleet-Modloader](https://github.com/logdot/Highfleet-Modloader),
[patchy-rs](https://github.com/logdot/patchy-rs),
[highfleet-rs](https://github.com/logdot/highfleet-rs), and the established hook
patterns of [highfleet-qol](https://github.com/logdot/highfleet-qol).
`highfleet` is pinned to reviewed full commit
`05c62078c6f896c141713a68b3991663e121b7bf`; compile-time assertions derive the
covered Node/Link/sentinel offsets without taking ownership of native memory.

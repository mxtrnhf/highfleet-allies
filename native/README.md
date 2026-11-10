# hf_native v0.1.4 — experimental native-engine acceleration

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
with its versioned name **`hf_native_v0.1.4.dll`**. No game execution is part of
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
| `link_index` | true | Exact `Link* -> cell` handle table (owns `AddNodeLink` + all removal sites); removes the list search |
| `tail_probe` | true | Newest-cell probe for `Link_UnlinkFromNode` and the `SLink` destructor (measured 58.5% hit rate) |
| `tail_probe_remove` | false | Newest-cell probe for `RemoveNodeLink`; measured 1.7% hit rate at 453.6 hops/call, so off by default |
| `parts_direct_append` | true | Direct in-place Link construction in `Body_CollectSubtreeLinks` when capacity is available (elides temporary Link register/unlink/free churn) |
| `contact_pool` | true | Scoped native contact allocation reuse |
| `contact_pool_capacity` | 4096 | Maximum cached free blocks per contact size class; 1..65536 |
| `scene_relink_probe` | true | Observer-only scene relink brackets and bounded shadow DFS; no ID index or changed native result |
| `media_lookup_probe` | true | Observer-only shadow media scans at seven call sites; no resource cache or changed native result |
| `diagnostics_interval_seconds` | 10 | Periodic atomic-counter snapshots in Modloader logs; 0 disables |

Logs identify the effective feature choices and byte-guard failures. Installation
fails before permanent patching when requested modules cannot validate their
supported native code. A log saying the DLL was found is not proof it installed.
The diagnostic thread reads counters only, never native game objects.

**v0.1.4 is an experiment build.** The two new probes repeat native searches to
measure their shape, then return the untouched native result. They intentionally
add work during scene loading; their counters are evidence, but load wall time
from this build is not an optimization A/B. No relink ID index or media pointer
cache is installed. The v0.1.3 link/topology/contact optimizations are unchanged.

## Test instructions

Use the same saved encounter, friendly/enemy heavy cruisers, configuration and
camera in each run. Keep the other DLL versions/settings fixed.

1. Restart with `enabled: true`. Confirm an `hf_native ... installed` line.
2. Load the same heavy-battle save, remain in it for at least 30 seconds, then
   use RETRY once. Exercise destruction, retreat and ship switching; verify
   native behavior and unchanged panel visuals. The load and RETRY exercise both
   new probes.
3. Report the newest Modloader log. Required observer health is `mismatch=0`,
   `incomplete=0`, `capped=0`, and no hash/root table-full growth. Relink
   `visits` versus `census nodes` decides whether an index can amortise; media
   `hops`, `bytes`, `distinct`, and per-site counts decide between no action, a
   repeat cache, or a future insert-time index.
4. For a behavior comparison, restart with `enabled: false` and repeat the same
   encounter. Do not compare probe-enabled load time with v0.1.3 as if the probes
   were accelerators.

For optional Linux CPU sampling, start this command and then load/RETRY the same
heavy battle during the 15-second window:

```bash
PID=$(pgrep -n -x Highfleet.exe)
perf record -e cpu-clock:u -F 999 --call-graph dwarf,16384 \
  -p "$PID" -o /tmp/opencode/highfleet-v014-relink.perf.data -- sleep 15
perf script -G -i /tmp/opencode/highfleet-v014-relink.perf.data \
  -F comm,tid,time,period,ip,sym,dso \
  > /tmp/opencode/highfleet-v014-relink.perf.txt
```

Select the actual game PID, not a Steam/Proton launcher. Keep the raw `.data`
file for native-address and sampled-register analysis. Windows stacks may not
unwind through Wine; leaf IPs remain useful. Use a different `v014-off` basename
for a disabled comparison and do not infer savings from different battles or
from the intentionally instrumented load duration.

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

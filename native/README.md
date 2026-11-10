# hf_native v0.1.5 — experimental native-engine acceleration

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
with its versioned name **`hf_native_v0.1.5.dll`**. No game execution is part of
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
| `scene_relink_probe` | true | Exact query counts/root classification plus bounded 1/256 shadow samples; no ID index or changed native result |
| `media_lookup_index` | true | Four-way positive animation-name cache; native fallback, singleton/head generation, destruction invalidation and 1/1024 native hit verification |
| `media_lookup_probe` | false | Explicit exhaustive shadow experiment at seven sites; overrides the index and adds traversal overhead |
| `host_query_probe` | true | Count unchanged `VirtualQuery` calls through pinned, already-loaded AI/ally/multicore imports; no region cache |
| `diagnostics_interval_seconds` | 10 | Periodic atomic-counter snapshots in Modloader logs; 0 disables |

Logs identify the effective feature choices and byte-guard failures. Installation
fails before permanent patching when requested modules cannot validate their
supported native code. A log saying the DLL was found is not proof it installed.
The diagnostic thread reads counters only, never native game objects.

The host-query line identifies coverage separately from volume. Its array order
is `(hf_ai 1.1.0, hf_ally 1.0.4, hf_mc 0.1.0)`. A missing DLL, different pinned
PE header/import layout, or pre-existing API hook is refused for that slot. QoL
loads after hf_native in the reviewed Modloader order and is not observed; Wine
internal/allocator queries and dynamically resolved calls are not observed.
Counts are not CPU-time measurements or per-feature attribution. Every observed
call still executes the original API and every existing memory guard remains.

**v0.1.5 accelerates positive animation lookups**, targeting the bucket responsible
for 99.18% of observed media-chain visits. It stores native's first-match answer,
including duplicate names, and compares names exactly on a hit. Unknown names
are not negatively cached. The table has 8192 slots, four probes and a 128-byte
name bound. Head/manager changes flush it; any virtual animation destruction
disables it before native release/free, and a single verification disagreement
also disables it. Native materialization, release and destruction still run.

The old exhaustive media probe is off. Relink's full second traversal on every
query and biased 32-root census are replaced by 1/256 bounded samples, root-class
and child-count histograms that cover outside-rebuild traffic too. An ID index
remains blocked on mutation/lifetime closure. No particle pool, rendering/GPU
change or guard removal is shipped: effect-update work is substantial, but its
inclusive allocator share has not been measured. Low self samples in allocator
wrappers do not establish an upper bound on their callees.

The four-way link table also fixes deletion-hole lookup: all four ways are
searched even if an earlier one is empty. This preserves the existing probe
bound while preventing a missed erase and stale cell mapping.

## Test instructions

Use the same saved encounter, friendly/enemy heavy cruisers, configuration and
camera in each run. Keep the other DLL versions/settings fixed.

1. Restart with `enabled: true`. Confirm an `hf_native ... installed` line.
2. Load the same heavy-battle save, remain in it for at least 30 seconds, then
   use RETRY once. Exercise destruction, retreat and ship switching; verify
   native behavior and unchanged panel visuals. Record cold-load and warm-RETRY
   results separately; the animation table warms across scene loads.
3. Report the newest Modloader log. `media_index` should show rising `hits`,
   `verify_mismatches: 0`, `auto_disabled: false`, `lifetime_disabled: false`
   during play. Destruction disabling at shutdown is expected. Misses, capped
   names, collisions or evictions are native fallbacks/lower hit rate, not wrong
   resources. Report unexpected epoch churn or disabling during play.
4. Relink `sample_mismatches` should be zero. `sample_nodes` describes only the
   sampled walks, not every native query; `sample_incomplete`, histogram overflow
   and sampled-root saturation limit interpretation. Host arrays must be read
   alongside `active`, in the order documented above.
5. To isolate the media accelerator, restart with `media_lookup_index: false`
   and `media_lookup_probe: false`, leaving all other switches unchanged. For a
   whole-mod behavior comparison use `enabled: false`. Do not use v0.1.4's
   deliberately inflated exhaustive-probe load time as an unbiased baseline.

For optional Linux CPU sampling, start this command and then load/RETRY the same
heavy battle during the 15-second window:

```bash
PID=$(pgrep -n -x Highfleet.exe)
LABEL=load  # use combat for a separate established-fighting capture
perf record -e cpu-clock:u -F 999 --call-graph dwarf,16384 \
  -p "$PID" -o "/tmp/opencode/highfleet-v015-${LABEL}.perf.data" -- sleep 15
perf script -G -i "/tmp/opencode/highfleet-v015-${LABEL}.perf.data" \
  -F comm,tid,time,period,ip,sym,dso \
  > "/tmp/opencode/highfleet-v015-${LABEL}.perf.txt"
```

Select the actual game PID, not a Steam/Proton launcher. Keep the raw `.data`
file for native-address and sampled-register analysis. Windows stacks may not
unwind through Wine; leaf IPs remain useful. Use a different `v015-off` basename
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

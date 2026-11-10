# Changelog

## 0.1.0 — 2026-11-10

- Initial experimental native-engine DLL for Steam 1.163 (`hf_native_v0.1.0.dll`).
- `links`: whole-body replacements for `RemoveNodeLink` (`0x140194fb0`), `Link_UnlinkFromNode` (`0x140194d20`), and `SLink_ScalarDeletingDestructor` (`0x140194e50`) with newest-cell tail probing (`length >= 2`) and hop/hit counters.
- `topology`: direct in-place `Link_ConstructFromBody` append in `Body_CollectSubtreeLinks` (`0x14001d02c` / `0x14001d031`), eliminating temporary `Link` registration/copy/unlink/free churn on the capacity-available path while preserving native growth and exception cleanup sites.
- `contacts`: scoped native-allocated free-list reuse for `b2CListener_PostSolve` (`0x48` record at `0x140284590`, `0x18` cell at `0x1402846ac`) and `SomethingPhysicTask` drain (`0x140286646`, `0x140286681`) via `C-unwind` near-CALL relays.
- Explicitly excluded / blocked: no panel or read-model hooks; no AI tick skipping or cadence changes; no persistent raw-cell `Link` cache or cross-frame retained `CalculateStats` category vectors (blocked on positive erase-completeness and writer-inventory proofs).

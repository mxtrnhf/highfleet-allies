//! Direct parts-list Link append: drops the temporary Link only when the
//! destination has capacity, while leaving the stock copy/growth/destructor at
//! their original native sites for exception cleanup.
//!
//! `Body_CollectSubtreeLinks` (`0x14001cfa0`) is the collector that fills the
//! parts vector for this rebuild; `Body_RebuildSubtreeList` (`0x140048500`)
//! calls it, and `CalculateStats` (`0x14007fcf0`) calls that at `0x14007fd6b`
//! with the creature's `CVec<Link>` at `+0x1330`. For every collected part the
//! stock sequence at `0x14001d024..0x14001d06b` is:
//!
//! ```text
//! Link temp;
//! Link_ConstructFromBody(&temp, body);          // 0x14001d02c, registers temp on body
//! if (dst->end == dst->capacity_end)
//!     LinkVec_EmplaceReallocate(dst, dst->end, &temp);   // 0x14001d05b
//! else {
//!     Link_CopyConstructor(dst->end, &temp);    // 0x14001d044, registers the element
//!     dst->end += 0x18;
//! }
//! Link_UnlinkFromNode(&temp);                   // 0x14001d066, scans body and frees temp's cell
//! ```
//!
//! Two disjoint detours implement the bounded equivalence:
//!
//! 1. `0x14001d02c` becomes a native `CALL` to a relay that does
//!    `mov r8, rbp; jmp callback`. The relay preserves the original native
//!    return address (`0x14001d031`) and passes `(temp, body, dst)` in
//!    `RCX/RDX/R8`. The callback either constructs the element directly in the
//!    destination slot and returns `true`, or calls **only** the original
//!    `Link_ConstructFromBody(temp, body)` and returns `false`.
//! 2. `0x14001d031` (`NOP` + `MOV RAX,[RBP+8]`) is replaced by a no-call
//!    trampoline: `TEST AL,AL`; `true` jumps to `0x14001d06b`, `false` replays
//!    the overwritten `MOV RAX,[RBP+8]` and jumps to `0x14001d036`.
//!
//! Therefore the stock copy, capacity growth and temporary destructor remain at
//! their original addresses on the false path (`0x14001d044`, `0x14001d05b`,
//! `0x14001d066`), including the compiler's `__CxxFrameHandler4` state for a
//! throw after the temporary exists. A throw from the constructor call keeps
//! the native return address `0x14001d031`, i.e. the stock constructor-call
//! site; whether the FH4 IP map excludes the temporary there is recorded as a
//! proof gap in `topology.md`, not assumed.
//!
//! On the true path the temporary is never constructed; the element is
//! constructed once at `dst->end` and `end` is advanced by `0x18`. The final
//! vector element (`LinkVFTABLE`, `target = body`, `id = body->m_id`), tail
//! order, vector length, per-body reference count, and Link id match the stock
//! copy path. The transient first registration has no observer: on the
//! capacity-available path the only calls between the stock construct and
//! unlink are the element copy and the native cell allocator.
//!
//! The only throwing operation in the callback is the same native cell `New`
//! the stock path already calls. The callback is `extern "C-unwind"` and the
//! native constructor pointer is `C-unwind`, so a constructor failure unwinds
//! through the original native return address. No Rust allocation, owning copy
//! or allocator substitution is performed.
//!
//! Not implemented here: retaining the derived vectors or reducing the dynamic
//! stats pass. That needs positive writer/lifecycle coverage for every
//! membership input and every Link destructor path; see `topology.md` for the
//! blocker.

use core::mem::{offset_of, size_of};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use highfleet::general::{CVec, Link, Node};
use patchy::{Condition, PatchSession, Trampoline};

use crate::call_hook;

/// `Body_CollectSubtreeLinks` entry, used for the prologue guard.
const COLLECT_FUNCTION: usize = 0x1_4001_cfa0;
const COLLECT_FUNCTION_PROLOGUE: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];

/// The stock append sequence. Both detours are validated against this full
/// window and the resume bytes before either patch is prepared.
const COLLECT_APPEND: usize = 0x1_4001_d024;
const COLLECT_APPEND_SEQUENCE: [u8; 0x47] = [
    0x48, 0x8b, 0xd6, 0x48, 0x8d, 0x4c, 0x24, 0x20, 0xe8, 0x9f, 0x7b, 0x17, 0x00, 0x90, 0x48, 0x8b,
    0x45, 0x08, 0x48, 0x3b, 0x45, 0x10, 0x74, 0x14, 0x48, 0x8d, 0x54, 0x24, 0x20, 0x48, 0x8b, 0xc8,
    0xe8, 0x37, 0x7b, 0x17, 0x00, 0x48, 0x83, 0x45, 0x08, 0x18, 0xeb, 0x11, 0x4c, 0x8d, 0x44, 0x24,
    0x20, 0x48, 0x8b, 0xd0, 0x48, 0x8b, 0xcd, 0xe8, 0xa0, 0x7a, 0xfe, 0xff, 0x90, 0x48, 0x8d, 0x4c,
    0x24, 0x20, 0xe8, 0xb5, 0x7c, 0x17, 0x00,
];

/// `CALL Link_ConstructFromBody`; replaced by a native CALL to the RBP-relay.
const COLLECT_APPEND_CALL: usize = 0x1_4001_d02c;
const COLLECT_APPEND_CALL_BYTES: [u8; 5] = [0xe8, 0x9f, 0x7b, 0x17, 0x00];
/// Native `CALL` return site. Stock bytes: `NOP` + `MOV RAX,[RBP+8]`.
const COLLECT_APPEND_RETURN: usize = 0x1_4001_d031;
const COLLECT_APPEND_RETURN_LEN: usize = 5;
/// The stock copy/growth branch begins here (`CMP RAX,[RBP+0x10]`).
const COLLECT_APPEND_COPY: usize = 0x1_4001_d036;
/// First instruction after the append sequence (`mov byte [rsi+0x388], 1`).
const COLLECT_APPEND_DIRECT: usize = 0x1_4001_d06b;
const COLLECT_APPEND_DIRECT_BYTES: [u8; 7] = [0xc6, 0x86, 0x88, 0x03, 0x00, 0x00, 0x01];

/// Native constructor the callback is allowed to call.
const LINK_CONSTRUCT_FROM_BODY: usize = 0x1_4019_4bd0;
const LINK_CONSTRUCT_FROM_BODY_PROLOGUE: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x8b, 0xc2, 0x48, 0x8d, 0x0d, 0x2d,
];

/// `CVec<Link>` is the covered native three-pointer header. `highfleet` keeps
/// the fields private and does not expose a way to advance the initialized
/// end, so this is the minimal read/write view needed at the append site.
#[repr(C)]
struct LinkVecHeader {
    begin: *mut Link,
    end: *mut Link,
    capacity_end: *mut Link,
}

const _: [(); 0x18] = [(); size_of::<Link>()];
const _: [(); 0x18] = [(); size_of::<CVec<Link>>()];
const _: [(); 0x18] = [(); size_of::<LinkVecHeader>()];
const _: [(); 0] = [(); offset_of!(LinkVecHeader, begin)];
const _: [(); 0x08] = [(); offset_of!(LinkVecHeader, end)];
const _: [(); 0x10] = [(); offset_of!(LinkVecHeader, capacity_end)];

type ConstructFromBody = unsafe extern "C-unwind" fn(*mut Link, *mut Node) -> *mut Link;

static DIRECT_APPENDS: AtomicU64 = AtomicU64::new(0);
static GROWTH_APPENDS: AtomicU64 = AtomicU64::new(0);
static FALLBACK_APPENDS: AtomicU64 = AtomicU64::new(0);
static ANOMALIES: AtomicU64 = AtomicU64::new(0);
static DISABLED: AtomicBool = AtomicBool::new(false);

/// Cheap process-lifetime counters for the parent's occasional logging.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Appends made directly into the vector slot; temporary Link elided.
    pub direct_appends: u64,
    /// Appends that returned false because the destination was at capacity.
    pub growth_appends: u64,
    /// Appends that returned false because the fast path was off.
    pub fallback_appends: u64,
    /// Unexpected vector header; fast path permanently disabled.
    pub anomalies: u64,
    /// Fast path disabled by an anomaly.
    pub disabled: bool,
}

/// Returns the current counters. One relaxed load each; no native memory reads.
pub fn snapshot() -> Counters {
    Counters {
        direct_appends: DIRECT_APPENDS.load(Ordering::Relaxed),
        growth_appends: GROWTH_APPENDS.load(Ordering::Relaxed),
        fallback_appends: FALLBACK_APPENDS.load(Ordering::Relaxed),
        anomalies: ANOMALIES.load(Ordering::Relaxed),
        disabled: DISABLED.load(Ordering::Relaxed),
    }
}

/// A direct slot is only used when the stock branch would take the copy path
/// (`end != capacity_end`) and the header is internally consistent. Anything
/// unexpected returns `false`, which leaves the stock copy/growth/destructor at
/// their original native sites.
unsafe fn direct_slot_is_plausible(header: *const LinkVecHeader) -> bool {
    let begin = (*header).begin as usize;
    let end = (*header).end as usize;
    let capacity_end = (*header).capacity_end as usize;
    if begin == 0 || end < begin || capacity_end <= end {
        return false;
    }
    let element_size = size_of::<Link>();
    (end - begin).is_multiple_of(element_size) && capacity_end - end >= element_size
}

/// Runs at the original `CALL Link_ConstructFromBody`, with the native return
/// address `0x14001d031` still on the stack. `temp` is the native stack slot
/// (`RCX`), `body` is `RDX`, `dst` is `R8` (relayed from `RBP`).
///
/// `true`: the element was constructed directly at `dst->end` and `end` was
/// advanced; the decision trampoline jumps to `0x14001d06b`.
/// `false`: only the original constructor ran; the stock copy/growth/destructor
/// continues from `0x14001d036`/`0x14001d05b`/`0x14001d066`.
unsafe extern "C-unwind" fn on_collect_append(
    temporary: *mut Link,
    body: *mut Node,
    dst: *mut CVec<Link>,
) -> bool {
    let construct: ConstructFromBody = core::mem::transmute(LINK_CONSTRUCT_FROM_BODY);

    if DISABLED.load(Ordering::Relaxed) {
        construct(temporary, body);
        FALLBACK_APPENDS.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    let header = dst.cast::<LinkVecHeader>();
    if (*header).end == (*header).capacity_end {
        construct(temporary, body);
        GROWTH_APPENDS.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    if !direct_slot_is_plausible(header) {
        DISABLED.store(true, Ordering::Relaxed);
        construct(temporary, body);
        ANOMALIES.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    let destination = (*header).end;
    construct(destination, body);
    (*header).end = destination.add(1);
    DIRECT_APPENDS.fetch_add(1, Ordering::Relaxed);
    true
}

/// Verifies the exact 1.163 part-subtree append and constructor before either
/// patch is prepared.
unsafe fn verify_native() -> Result<(), String> {
    for (name, address, expected) in [
        (
            "Body_CollectSubtreeLinks prologue",
            COLLECT_FUNCTION,
            &COLLECT_FUNCTION_PROLOGUE[..],
        ),
        (
            "Body_CollectSubtreeLinks append sequence",
            COLLECT_APPEND,
            &COLLECT_APPEND_SEQUENCE[..],
        ),
        (
            "Body_CollectSubtreeLinks append resume",
            COLLECT_APPEND_DIRECT,
            &COLLECT_APPEND_DIRECT_BYTES[..],
        ),
        (
            "Link_ConstructFromBody prologue",
            LINK_CONSTRUCT_FROM_BODY,
            &LINK_CONSTRUCT_FROM_BODY_PROLOGUE[..],
        ),
    ] {
        let found = std::slice::from_raw_parts(address as *const u8, expected.len());
        if found != expected {
            return Err(format!(
                "{name} at {address:#x} has unexpected bytes {found:02x?}, expected {expected:02x?}"
            ));
        }
    }
    Ok(())
}

/// Prepares the two disjoint part-subtree detours on the supported 1.163 image.
///
/// `enabled == false` leaves the session untouched. The full append window,
/// resume, collector prologue and constructor prologue are validated before
/// either patch is added. The first patch uses the parent-owned
/// `call_hook::prepare_call_with_rbp_argument`, which replaces the native CALL
/// with another native CALL to a relay (`mov r8, rbp; jmp callback`) so the
/// original return address `0x14001d031` stays on the stack. The second patch
/// is a no-call decision trampoline at that return site.
///
/// # Safety
///
/// Must run during mod initialization on the game thread, after the parent has
/// validated the executable identity and base, and before the part-subtree
/// collector runs. The detours are permanent; the disabled path inside the
/// callback restores the stock constructor/copy/growth/destructor path per call
/// but does not unpatch.
pub unsafe fn prepare(session: &mut PatchSession, enabled: bool) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }

    verify_native()?;

    // Patch 1: preserve the native CALL return address. The relay only sets
    // R8 = RBP; RCX/RDX already hold the temporary slot and the Body.
    call_hook::prepare_call_with_rbp_argument(
        session,
        COLLECT_APPEND_CALL,
        &COLLECT_APPEND_CALL_BYTES,
        on_collect_append as *const (),
    )?;

    // Patch 2: no calls in this trampoline. AL is the callback's bool result.
    let mut trampoline = Trampoline::new();
    trampoline.bytes(&[0x84, 0xc0]); // test al, al
    let stock = trampoline.new_label();
    trampoline.jump_if(Condition::Equal, stock);
    trampoline.absolute_jump(COLLECT_APPEND_DIRECT); // true: skip copy/unlink
    trampoline.bind(stock).map_err(|error| error.to_string())?;
    trampoline.bytes(&[0x48, 0x8b, 0x45, 0x08]); // false: replay MOV RAX,[RBP+8]
    trampoline.absolute_jump(COLLECT_APPEND_COPY);

    session
        .detour_trampoline(COLLECT_APPEND_RETURN, COLLECT_APPEND_RETURN_LEN, trampoline)
        .map_err(|error| format!("part-subtree append decision detour: {error}"))?;

    DISABLED.store(false, Ordering::Relaxed);
    Ok(())
}

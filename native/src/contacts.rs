//! Reuse of the two native `b2CListener_PostSolve` contact-record allocations.
//!
//! Every accepted contact allocates a 0x48-byte record and a 0x18-byte list
//! cell in `b2CListener_PostSolve`, then releases both in the
//! `SomethingPhysicTask` contact-list drain. The four call sites were verified
//! live in `Highfleet.exe.unpacked.exe v1.163` on 2026-11-10:
//!
//! | Role | Call site | Instruction |
//! |---|---|---|
//! | record allocate | `0x140284590` | `CALL New` (`ECX = 0x48`) |
//! | cell allocate | `0x1402846ac` | `CALL New` (`ECX = 0x18`) |
//! | record free | `0x140286646` | `CALL free` (`RCX = record`, `EDX = 0x48`) |
//! | cell free | `0x140286681` | `CALL free` (`RCX = cell`, `EDX = 0x18`) |
//!
//! The pool is a bounded, DLL-private stack of block addresses. It stores no
//! metadata inside the native blocks: a recycled block is the exact pointer the
//! native `New` returned, with its original alignment and its previous contents.
//! Blocks enter the pool only when the normal drain free hook consumes the
//! native `free` instead of calling it, and leave it when one of the two `New`
//! hooks reuses them. A pointer freed through any other path is therefore never
//! cached, and no bulk reset or per-pointer ownership registry is needed.
//!
//! Each site is patched with `crate::call_hook::prepare_call`: an `E8 CALL` to a
//! near RX relay that tail-jumps the Rust callback. The original return address
//! (`site + 5`) therefore stays on the stack and the callbacks are
//! `extern "C-unwind"`, so a native `bad_alloc` from `New` can unwind through
//! the callback into the original caller. The callbacks never hold the pool
//! lock across a native call.
//!
//! Correctness argument and residual risks are in the 2026-11-10 build report
//! filed with the native-DLL run (`contacts.md`).

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use patchy::PatchSession;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

use crate::call_hook;

/// `b2CListener_PostSolve` record allocation: `mov ecx,0x48; call New`.
const ALLOC_RECORD_SITE: usize = 0x1_4028_4590;
/// `b2CListener_PostSolve` list-cell allocation: `mov ecx,0x18; call New`.
const ALLOC_CELL_SITE: usize = 0x1_4028_46ac;
/// `SomethingPhysicTask` record release: `mov edx,0x48; mov rcx,rbx; call free`.
const FREE_RECORD_SITE: usize = 0x1_4028_6646;
/// `SomethingPhysicTask` list-cell release: `mov edx,0x18; mov rcx,r8; call free`.
const FREE_CELL_SITE: usize = 0x1_4028_6681;

/// Start of the record-allocation guard, at its `mov ecx,0x48`.
const ALLOC_RECORD_GUARD_ADDR: usize = 0x1_4028_458b;
/// Start of the cell-allocation guard, at its `mov ecx,0x18`.
const ALLOC_CELL_GUARD_ADDR: usize = 0x1_4028_46a7;
/// Start of the record-free guard, at its first `lea rcx,[rbx+0x18]`.
const FREE_RECORD_GUARD_ADDR: usize = 0x1_4028_662d;
/// Start of the cell-free guard, at its `mov edx,0x18`.
const FREE_CELL_GUARD_ADDR: usize = 0x1_4028_666a;

/// Exact five call bytes, revalidated by `call_hook::prepare_call`.
const ALLOC_RECORD_CALL: [u8; 5] = [0xe8, 0xff, 0xfa, 0x0a, 0x00];
const ALLOC_CELL_CALL: [u8; 5] = [0xe8, 0xe3, 0xf9, 0x0a, 0x00];
const FREE_RECORD_CALL: [u8; 5] = [0xe8, 0x85, 0xda, 0x0a, 0x00];
const FREE_CELL_CALL: [u8; 5] = [0xe8, 0x4a, 0xda, 0x0a, 0x00];

/// Native scalar `operator new` (`New(size_t)`), the fallback allocator.
const NATIVE_NEW: usize = 0x1_4033_4094;
/// Native `free` thunk, the fallback deallocator. Unchanged by other native hooks.
const NATIVE_FREE: usize = 0x1_4033_40d0;

const RECORD_SIZE: usize = 0x48;
const CELL_SIZE: usize = 0x18;

/// Largest accepted `contact_pool_capacity`; the config validates 1..=65536.
const MAX_CAPACITY: usize = 65_536;

/// Bytes at `0x14028458b`: size prefix, call, and the two stores of the result.
const ALLOC_RECORD_GUARD: [u8; 18] = [
    0xb9, 0x48, 0x00, 0x00, 0x00, // mov ecx,0x48
    0xe8, 0xff, 0xfa, 0x0a, 0x00, // call New
    0x48, 0x8b, 0xf8, // mov rdi,rax
    0x48, 0x89, 0x44, 0x24, 0x30, // mov [rsp+0x30],rax
];

/// Bytes at `0x1402846a7`: node New, the store into `node+0x10`, and the list
/// count increment that identify the contact-list cell.
const ALLOC_CELL_GUARD: [u8; 21] = [
    0xb9, 0x18, 0x00, 0x00, 0x00, // mov ecx,0x18
    0xe8, 0xe3, 0xf9, 0x0a, 0x00, // call New
    0x48, 0x89, 0x78, 0x10, // mov [rax+0x10],rdi
    0x48, 0xff, 0x05, 0x94, 0xf5, 0x7a, 0x03, // inc qword ptr [0x143a33c50]
];

/// Bytes at `0x14028662d`: both `Link_UnlinkFromNode` calls and the record free.
/// The native destructors run before the pooled block is released, unchanged.
const FREE_RECORD_GUARD: [u8; 30] = [
    0x48, 0x8d, 0x4b, 0x18, // lea rcx,[rbx+0x18]
    0xe8, 0xea, 0xe6, 0xf0, 0xff, // call Link_UnlinkFromNode
    0x48, 0x8b, 0xcb, // mov rcx,rbx
    0xe8, 0xe2, 0xe6, 0xf0, 0xff, // call Link_UnlinkFromNode
    0xba, 0x48, 0x00, 0x00, 0x00, // mov edx,0x48
    0x48, 0x8b, 0xcb, // mov rcx,rbx
    0xe8, 0x85, 0xda, 0x0a, 0x00, // call free
];

/// Bytes at `0x14028666a`: the circular-list unlink and the cell free.
const FREE_CELL_GUARD: [u8; 28] = [
    0xba, 0x18, 0x00, 0x00, 0x00, // mov edx,0x18
    0x49, 0x8b, 0x40, 0x08, // mov rax,[r8+0x8]
    0x48, 0x89, 0x08, // mov [rax],rcx
    0x49, 0x8b, 0x40, 0x08, // mov rax,[r8+0x8]
    0x48, 0x89, 0x41, 0x08, // mov [rcx+0x8],rax
    0x49, 0x8b, 0xc8, // mov rcx,r8
    0xe8, 0x4a, 0xda, 0x0a, 0x00, // call free
];

/// Native `New` entry: `push rbx; sub rsp,0x20; mov rbx,rcx; jmp ...`.
const NATIVE_NEW_GUARD: [u8; 11] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0xeb, 0x0f,
];

/// Native `free` thunk: `jmp 0x140334e90`.
const NATIVE_FREE_GUARD: [u8; 5] = [0xe9, 0xbb, 0x0d, 0x00, 0x00];

type NativeNew = unsafe extern "C-unwind" fn(usize) -> *mut u8;
type NativeFree = unsafe extern "C-unwind" fn(usize, usize);

/// Counters and pool lengths readable without taking the pool lock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// True once `prepare` validated every site and added every patch.
    pub active: bool,
    /// Configured per-class capacity.
    pub capacity: usize,
    /// Records currently cached for reuse.
    pub records_pooled: usize,
    /// List cells currently cached for reuse.
    pub cells_pooled: usize,
    /// Bytes held by cached blocks (`0x48` or `0x18` each).
    pub pooled_bytes: usize,
    /// Record allocations served from the pool.
    pub record_hits: u64,
    /// Cell allocations served from the pool.
    pub cell_hits: u64,
    /// Record allocations forwarded to native `New`.
    pub record_native: u64,
    /// Cell allocations forwarded to native `New`.
    pub cell_native: u64,
    /// Record frees retained for reuse instead of calling native `free`.
    pub record_cached: u64,
    /// Cell frees retained for reuse instead of calling native `free`.
    pub cell_cached: u64,
    /// Record frees forwarded to native `free` (pool full or foreign state).
    pub record_forwarded: u64,
    /// Cell frees forwarded to native `free` (pool full or foreign state).
    pub cell_forwarded: u64,
    pub native_new_calls: u64,
    pub native_free_calls: u64,
    /// Hooks that skipped the pool because another thread held its lock.
    pub lock_busy: u64,
    /// Hooks that fell back because the calling thread was not the prepare thread.
    pub foreign_thread: u64,
}

#[derive(Default)]
struct Counters {
    record_hits: AtomicU64,
    cell_hits: AtomicU64,
    record_native: AtomicU64,
    cell_native: AtomicU64,
    record_cached: AtomicU64,
    cell_cached: AtomicU64,
    record_forwarded: AtomicU64,
    cell_forwarded: AtomicU64,
    native_new_calls: AtomicU64,
    native_free_calls: AtomicU64,
    lock_busy: AtomicU64,
    foreign_thread: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            active: POOL.main_thread.load(Ordering::Relaxed) != 0,
            capacity: POOL.capacity.load(Ordering::Relaxed),
            records_pooled: POOL.record_len.load(Ordering::Acquire),
            cells_pooled: POOL.cell_len.load(Ordering::Acquire),
            pooled_bytes: pooled_bytes(),
            record_hits: self.record_hits.load(Ordering::Relaxed),
            cell_hits: self.cell_hits.load(Ordering::Relaxed),
            record_native: self.record_native.load(Ordering::Relaxed),
            cell_native: self.cell_native.load(Ordering::Relaxed),
            record_cached: self.record_cached.load(Ordering::Relaxed),
            cell_cached: self.cell_cached.load(Ordering::Relaxed),
            record_forwarded: self.record_forwarded.load(Ordering::Relaxed),
            cell_forwarded: self.cell_forwarded.load(Ordering::Relaxed),
            native_new_calls: self.native_new_calls.load(Ordering::Relaxed),
            native_free_calls: self.native_free_calls.load(Ordering::Relaxed),
            lock_busy: self.lock_busy.load(Ordering::Relaxed),
            foreign_thread: self.foreign_thread.load(Ordering::Relaxed),
        }
    }
}

fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

fn pooled_bytes() -> usize {
    POOL.record_len.load(Ordering::Acquire) * RECORD_SIZE
        + POOL.cell_len.load(Ordering::Acquire) * CELL_SIZE
}

struct Pool {
    /// Non-blocking guard for both free lists. A contended hook uses native paths.
    lock: AtomicBool,
    /// 0 until a successful `prepare`; keeps pushes out of the array.
    capacity: AtomicUsize,
    /// Thread that called `prepare`; hooks on other threads use native paths.
    main_thread: AtomicU32,
    record_len: AtomicUsize,
    cell_len: AtomicUsize,
    records: UnsafeCell<[usize; MAX_CAPACITY]>,
    cells: UnsafeCell<[usize; MAX_CAPACITY]>,
    counters: Counters,
}

// SAFETY: every access to the `UnsafeCell` arrays happens while `lock` is held
// by a hook that already passed the prepare-thread check (or by `prepare`
// itself). Foreign threads take the native path before `lock_pool`. The
// atomics are independently thread-safe.
unsafe impl Sync for Pool {}

static POOL: Pool = Pool {
    lock: AtomicBool::new(false),
    capacity: AtomicUsize::new(0),
    main_thread: AtomicU32::new(0),
    record_len: AtomicUsize::new(0),
    cell_len: AtomicUsize::new(0),
    records: UnsafeCell::new([0; MAX_CAPACITY]),
    cells: UnsafeCell::new([0; MAX_CAPACITY]),
    counters: Counters {
        record_hits: AtomicU64::new(0),
        cell_hits: AtomicU64::new(0),
        record_native: AtomicU64::new(0),
        cell_native: AtomicU64::new(0),
        record_cached: AtomicU64::new(0),
        cell_cached: AtomicU64::new(0),
        record_forwarded: AtomicU64::new(0),
        cell_forwarded: AtomicU64::new(0),
        native_new_calls: AtomicU64::new(0),
        native_free_calls: AtomicU64::new(0),
        lock_busy: AtomicU64::new(0),
        foreign_thread: AtomicU64::new(0),
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Record,
    Cell,
}

impl Class {
    fn size(self) -> usize {
        match self {
            Self::Record => RECORD_SIZE,
            Self::Cell => CELL_SIZE,
        }
    }

    fn len(self) -> &'static AtomicUsize {
        match self {
            Self::Record => &POOL.record_len,
            Self::Cell => &POOL.cell_len,
        }
    }
}

/// Guard that releases the pool lock on drop.
struct PoolLock;

impl Drop for PoolLock {
    fn drop(&mut self) {
        POOL.lock.store(false, Ordering::Release);
    }
}

/// Takes the pool lock without blocking; `None` means the caller uses native paths.
fn lock_pool() -> Option<PoolLock> {
    match POOL
        .lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
    {
        Ok(_) => Some(PoolLock),
        Err(_) => {
            bump(&POOL.counters.lock_busy);
            None
        }
    }
}

/// Free-list storage for one class.
///
/// # Safety
///
/// The caller must hold the pool lock; this yields an exclusive reference to
/// storage that no other thread may touch until the lock is released.
unsafe fn slots(class: Class) -> &'static mut [usize] {
    let storage: &'static mut [usize; MAX_CAPACITY] = match class {
        Class::Record => &mut *POOL.records.get(),
        Class::Cell => &mut *POOL.cells.get(),
    };
    storage
}

/// Moves a native-allocated block into the pool. Returns false when it must be freed.
unsafe fn cache(class: Class, block: usize) -> bool {
    let Some(_guard) = lock_pool() else {
        return false;
    };
    let capacity = POOL.capacity.load(Ordering::Relaxed);
    let len = class.len().load(Ordering::Relaxed);
    if len >= capacity {
        return false;
    }
    slots(class)[len] = block;
    class.len().store(len + 1, Ordering::Release);
    true
}

/// Takes one block of exactly `class` out of the pool, if available.
unsafe fn reuse(class: Class) -> Option<usize> {
    let _guard = lock_pool()?;
    let len = class.len().load(Ordering::Relaxed);
    let reduced_len = len.checked_sub(1)?;
    let storage = slots(class);
    let block = storage[reduced_len];
    if block == 0 {
        return None;
    }
    storage[reduced_len] = 0;
    class.len().store(reduced_len, Ordering::Release);
    Some(block)
}

/// Whether this hook runs on the thread that called `prepare`.
fn on_main_thread() -> bool {
    let main_thread = POOL.main_thread.load(Ordering::Relaxed);
    main_thread != 0 && unsafe { GetCurrentThreadId() } == main_thread
}

/// Calls the native allocator with the call site's original argument.
///
/// # Safety
///
/// `NATIVE_NEW` must still be the verified `New(size_t)` entry, which `prepare`
/// checks.
unsafe fn native_new(size: usize) -> *mut u8 {
    bump(&POOL.counters.native_new_calls);
    let allocate: NativeNew = std::mem::transmute(NATIVE_NEW);
    allocate(size)
}

/// Calls the native deallocator with the call site's original arguments.
///
/// # Safety
///
/// `NATIVE_FREE` must still be the verified `free` thunk, which `prepare`
/// checks, and `block` must be a live native allocation.
unsafe fn native_free(block: usize, size: usize) {
    bump(&POOL.counters.native_free_calls);
    let release: NativeFree = std::mem::transmute(NATIVE_FREE);
    release(block, size);
}

/// Allocation replacement for one class, matching `New(size_t) -> void*`.
///
/// A pool hit never calls native code. On a miss `reuse` has already released
/// the lock, so a throwing native `New` unwinds without a live pool guard.
unsafe fn allocate(class: Class, size: usize) -> *mut u8 {
    if size != class.size() {
        bump(match class {
            Class::Record => &POOL.counters.record_native,
            Class::Cell => &POOL.counters.cell_native,
        });
        return native_new(size);
    }
    if !on_main_thread() {
        bump(&POOL.counters.foreign_thread);
        bump(match class {
            Class::Record => &POOL.counters.record_native,
            Class::Cell => &POOL.counters.cell_native,
        });
        return native_new(size);
    }

    let Some(block) = reuse(class) else {
        bump(match class {
            Class::Record => &POOL.counters.record_native,
            Class::Cell => &POOL.counters.cell_native,
        });
        return native_new(size);
    };
    bump(match class {
        Class::Record => &POOL.counters.record_hits,
        Class::Cell => &POOL.counters.cell_hits,
    });
    block as *mut u8
}

/// Free replacement for one class, matching `free(void*)` plus the call site's `EDX`.
///
/// `cache` releases the pool lock before the native `free` fallback is called.
unsafe fn release(class: Class, block: usize, size: usize) {
    // The size check only keeps foreign callers out; native `free` is always the
    // correct fallback because every block this hook can see came from the
    // native allocator. Pool state never decides *how* to free a block.
    if block == 0 || size != class.size() {
        bump(forwarded_counter(class));
        native_free(block, size);
        return;
    }

    if !on_main_thread() {
        bump(&POOL.counters.foreign_thread);
        bump(forwarded_counter(class));
        native_free(block, size);
        return;
    }

    if cache(class, block) {
        bump(match class {
            Class::Record => &POOL.counters.record_cached,
            Class::Cell => &POOL.counters.cell_cached,
        });
        return;
    }

    bump(forwarded_counter(class));
    native_free(block, size);
}

fn forwarded_counter(class: Class) -> &'static AtomicU64 {
    match class {
        Class::Record => &POOL.counters.record_forwarded,
        Class::Cell => &POOL.counters.cell_forwarded,
    }
}

/// Call-site replacement for the record `New(0x48)`.
unsafe extern "C-unwind" fn allocate_record(size: usize) -> *mut u8 {
    allocate(Class::Record, size)
}

/// Call-site replacement for the cell `New(0x18)`.
unsafe extern "C-unwind" fn allocate_cell(size: usize) -> *mut u8 {
    allocate(Class::Cell, size)
}

/// Call-site replacement for the record `free(record)` (`EDX = 0x48`).
unsafe extern "C-unwind" fn free_record(block: usize, size: usize) {
    release(Class::Record, block, size)
}

/// Call-site replacement for the cell `free(cell)` (`EDX = 0x18`).
unsafe extern "C-unwind" fn free_cell(block: usize, size: usize) {
    release(Class::Cell, block, size)
}

/// Reads a small native byte range and compares it with the verified 1.163 bytes.
unsafe fn check_bytes(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let found = std::slice::from_raw_parts(address as *const u8, expected.len());
    if found != expected {
        return Err(format!(
            "{label} at {address:#x} has unexpected bytes {found:02x?} (expected {expected:02x?}); contact pool not installed"
        ));
    }
    Ok(())
}

/// Verifies that the `E8` at `site` still calls `expected`.
unsafe fn check_call_target(site: usize, expected: usize, label: &str) -> Result<(), String> {
    let opcode = (site as *const u8).read();
    if opcode != 0xe8 {
        return Err(format!(
            "{label} at {site:#x} is not a near call (opcode {opcode:#04x}); contact pool not installed"
        ));
    }
    let displacement = std::ptr::read_unaligned((site + 1) as *const i32) as isize;
    let target = site.wrapping_add(5).wrapping_add(displacement as usize);
    if target != expected {
        return Err(format!(
            "{label} at {site:#x} calls {target:#x}, expected {expected:#x}; contact pool not installed"
        ));
    }
    Ok(())
}

/// Validates every byte the module depends on before a single patch is prepared.
unsafe fn validate() -> Result<(), String> {
    check_bytes(
        ALLOC_RECORD_GUARD_ADDR,
        &ALLOC_RECORD_GUARD,
        "contact record allocation",
    )?;
    check_call_target(ALLOC_RECORD_SITE, NATIVE_NEW, "contact record allocation")?;
    check_bytes(
        ALLOC_CELL_GUARD_ADDR,
        &ALLOC_CELL_GUARD,
        "contact list-cell allocation",
    )?;
    check_call_target(ALLOC_CELL_SITE, NATIVE_NEW, "contact list-cell allocation")?;
    check_bytes(
        FREE_RECORD_GUARD_ADDR,
        &FREE_RECORD_GUARD,
        "contact record release",
    )?;
    check_call_target(FREE_RECORD_SITE, NATIVE_FREE, "contact record release")?;
    check_bytes(
        FREE_CELL_GUARD_ADDR,
        &FREE_CELL_GUARD,
        "contact list-cell release",
    )?;
    check_call_target(FREE_CELL_SITE, NATIVE_FREE, "contact list-cell release")?;
    check_bytes(NATIVE_NEW, &NATIVE_NEW_GUARD, "native New entry")?;
    check_bytes(NATIVE_FREE, &NATIVE_FREE_GUARD, "native free thunk")?;
    Ok(())
}

/// Prepares all four call-site replacements. Validation happens first, so an
/// error leaves the session untouched.
///
/// # Safety
///
/// Must be called once, after the parent has verified the 1.163 image base,
/// before native code runs on the game's main thread. All guarded addresses
/// must be mapped; a wrong image base faults during validation rather than
/// returning an error. The parent installs the session permanently; no live
/// unload exists.
pub unsafe fn prepare(session: &mut PatchSession, capacity: usize) -> Result<(), String> {
    if !(1..=MAX_CAPACITY).contains(&capacity) {
        return Err(format!(
            "contact_pool_capacity {capacity} outside 1..={MAX_CAPACITY}"
        ));
    }

    validate()?;

    let sites: [(usize, [u8; 5], *const (), usize, &str); 4] = [
        (
            ALLOC_RECORD_SITE,
            ALLOC_RECORD_CALL,
            allocate_record as *const (),
            NATIVE_NEW,
            "contact record allocate",
        ),
        (
            ALLOC_CELL_SITE,
            ALLOC_CELL_CALL,
            allocate_cell as *const (),
            NATIVE_NEW,
            "contact list-cell allocate",
        ),
        (
            FREE_RECORD_SITE,
            FREE_RECORD_CALL,
            free_record as *const (),
            NATIVE_FREE,
            "contact record free",
        ),
        (
            FREE_CELL_SITE,
            FREE_CELL_CALL,
            free_cell as *const (),
            NATIVE_FREE,
            "contact list-cell free",
        ),
    ];

    for (site, call, hook, target, label) in sites {
        // The full semantic window was checked by `validate`; re-checking the
        // call target here narrows a foreign-patch race before the snapshot.
        check_call_target(site, target, label)?;
        // Parent helper: an E8 CALL to a near relay that tail-jumps the
        // C-unwind callback, so site+5 stays on the stack for native unwinding.
        call_hook::prepare_call(session, site, &call, hook)
            .map_err(|error| format!("{label} at {site:#x}: {error}"))?;
    }

    POOL.record_len.store(0, Ordering::Relaxed);
    POOL.cell_len.store(0, Ordering::Relaxed);
    POOL.capacity.store(capacity, Ordering::Release);
    POOL.main_thread
        .store(GetCurrentThreadId(), Ordering::Release);
    Ok(())
}

/// Returns the pool counters. Every read is atomic, so a diagnostic thread may
/// call this while the game thread is inserting contacts.
pub fn snapshot() -> Snapshot {
    POOL.counters.snapshot()
}

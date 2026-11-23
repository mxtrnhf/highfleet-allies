//! Diagnostics for tester logs.
//!
//! * Always on: the version banner, a panic hook and an exception handler that write a crash report (what
//!   crashed, in which module and at which Ghidra address, and whether a hf_ally hook was running) into the log
//!   and flush it. The modloader buffers its log file (flexi_logger `BufferAndFlush`), so without a flush the
//!   last second before a crash is lost.
//! * `logging.debug_log` (runtime switch, default off): a second, unbuffered file
//!   `Modloader/logs/hf_ally_megalog_<time>.log` with a heartbeat, every click, every change of the game's scene
//!   state and every spawn, a watchdog that flushes the normal log every 300 ms, and a minidump
//!   (`hf_ally_crash_<time>.dmp`) on an unhandled crash.

use std::{
    ffi::c_void,
    fmt::Write as _,
    fs::File,
    io::Write as _,
    mem::offset_of,
    panic,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::native_layout::Creature;

use windows_sys::Win32::System::{
    Diagnostics::Debug::{
        AddVectoredExceptionHandler, MiniDumpWriteDump, RtlCaptureStackBackTrace,
        SetUnhandledExceptionFilter, EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION,
    },
    LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    },
    Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS},
    Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId},
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD: &str = env!("HF_ALLY_BUILD");

/// The modloader may buffer its log: flush after the lines that matter for a crash report.
pub(crate) fn flush_log() {
    log::logger().flush();
}
/// Debug mode (megalog file, watchdog, crash minidump, verbose lines), set from `logging.debug_log`.
/// Off unless the JSON enables it; the legacy `megalog` cargo feature does not force it.
static DEBUG_ON: AtomicBool = AtomicBool::new(false);
static MEGALOG_STARTED: AtomicBool = AtomicBool::new(false);

#[inline]
pub fn debug() -> bool {
    DEBUG_ON.load(Ordering::Relaxed)
}

/// The first line a tester's log must show: version, build time and whether verbose debug mode is on.
pub fn banner() -> String {
    format!(
        "hf_ally v{VERSION} (build {BUILD} UTC){}",
        if debug() {
            " DEBUG MODE (debug_log): verbose, writes a large extra log file and a crash dump"
        } else {
            ""
        }
    )
}

static START: OnceLock<Instant> = OnceLock::new();
static FILE: Mutex<Option<File>> = Mutex::new(None);

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn write_file(msg: &str, in_crash: bool) {
    let t = START
        .get()
        .map(|s| s.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    let line = format!("[+{t:9.3}] {msg}\n");
    // a crash may have happened while another thread held the lock: never wait for it there
    let guard = if in_crash {
        FILE.try_lock().ok()
    } else {
        FILE.lock().ok()
    };
    if let Some(mut guard) = guard {
        if let Some(file) = guard.as_mut() {
            let _ = file.write_all(line.as_bytes()); // unbuffered: it reaches the OS even if the process dies next
        }
    }
}

/// A line for the megalog only (debug mode). Use through the `crumb!` macro so the string is not built otherwise.
pub fn crumb(msg: &str) {
    if debug() {
        write_file(msg, false);
    }
}

// --- which hf_ally hook is running (read by the crash report) ---

static HOOK_PTR: AtomicUsize = AtomicUsize::new(0);
static HOOK_LEN: AtomicUsize = AtomicUsize::new(0);

pub struct Scope;
/// Marks "inside hf_ally hook `name`" until the returned guard is dropped.
pub fn scope(name: &'static str) -> Scope {
    HOOK_LEN.store(name.len(), Ordering::Relaxed);
    HOOK_PTR.store(name.as_ptr() as usize, Ordering::Relaxed);
    Scope
}
impl Drop for Scope {
    fn drop(&mut self) {
        HOOK_PTR.store(0, Ordering::Relaxed);
    }
}
fn current_hook() -> String {
    let (ptr, len) = (
        HOOK_PTR.load(Ordering::Relaxed),
        HOOK_LEN.load(Ordering::Relaxed),
    );
    if ptr == 0 {
        return "none (the game's own code)".to_string();
    }
    unsafe {
        String::from_utf8_lossy(std::slice::from_raw_parts(ptr as *const u8, len)).into_owned()
    }
}

// --- crash report ---

/// Where `addr` lives: `module.dll+0xoffset`, plus the Ghidra address for the game's own exe.
unsafe fn where_is(addr: usize) -> String {
    const MODULE_PATH_CAPACITY: usize = 260;

    let mut module = std::ptr::null_mut();
    let flags =
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    if GetModuleHandleExW(flags, addr as *const u16, &mut module) == 0 || module.is_null() {
        return format!("{addr:#x} (no module)");
    }

    let mut path_buffer = [0u16; MODULE_PATH_CAPACITY];
    let written = GetModuleFileNameW(module, path_buffer.as_mut_ptr(), path_buffer.len() as u32);
    let path = String::from_utf16_lossy(&path_buffer[..(written as usize).min(path_buffer.len())]);
    let name = path.rsplit(['\\', '/']).next().unwrap_or("?");
    let offset = addr - module as usize;
    if module as usize == crate::native::IMAGE_BASE {
        format!("{name}+{offset:#x} (Ghidra {addr:#x})")
    } else {
        format!("{name}+{offset:#x}")
    }
}

unsafe fn readable(addr: usize) -> bool {
    let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
    if VirtualQuery(
        addr as *const c_void,
        &mut info,
        std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
    ) == 0
    {
        return false;
    }
    info.State == MEM_COMMIT
        && info.Protect & (PAGE_NOACCESS | PAGE_GUARD) == 0
        && info.Protect != 0
}

fn emit(report: &str, in_crash: bool) {
    for line in report.lines() {
        write_file(line, in_crash);
    }
    log::error!("{report}");
    flush_log();
}

unsafe fn report(kind: &str, info: *const EXCEPTION_POINTERS) {
    const MAX_BACKTRACE_FRAMES: usize = 48;
    const STACK_SCAN_SLOTS: usize = 384;
    const MAX_STACK_HITS: usize = 40;

    // SAFETY: Both Win32 exception callbacks receive these records from the OS; the references
    // stay inside this synchronous callback and are never retained or used to modify the context.
    let rec = &*(*info).ExceptionRecord;
    let ctx = &*(*info).ContextRecord;
    let code = rec.ExceptionCode as u32;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "CRASH ({kind}) in hf_ally v{VERSION} build {BUILD}: exception {code:#010x} at {} | thread {} | hf_ally hook running: {}",
        where_is(rec.ExceptionAddress as usize),
        GetCurrentThreadId(),
        current_hook()
    );
    if code == 0xC000_0005 && rec.NumberParameters >= 2 {
        let how = match rec.ExceptionInformation[0] {
            0 => "reading",
            1 => "writing",
            _ => "executing",
        };
        let _ = writeln!(
            s,
            "CRASH access violation {how} address {:#x}",
            rec.ExceptionInformation[1]
        );
    }
    let _ = writeln!(
        s,
        "CRASH regs rax={:#x} rbx={:#x} rcx={:#x} rdx={:#x} rsi={:#x} rdi={:#x} rbp={:#x} rsp={:#x}",
        ctx.Rax, ctx.Rbx, ctx.Rcx, ctx.Rdx, ctx.Rsi, ctx.Rdi, ctx.Rbp, ctx.Rsp
    );
    let _ = writeln!(
        s,
        "CRASH regs r8={:#x} r9={:#x} r10={:#x} r11={:#x} r12={:#x} r13={:#x} r14={:#x} r15={:#x} rip={:#x}",
        ctx.R8, ctx.R9, ctx.R10, ctx.R11, ctx.R12, ctx.R13, ctx.R14, ctx.R15, ctx.Rip
    );

    let mut frames = [std::ptr::null_mut::<c_void>(); MAX_BACKTRACE_FRAMES];
    let captured = RtlCaptureStackBackTrace(
        0,
        frames.len() as u32,
        frames.as_mut_ptr(),
        std::ptr::null_mut(),
    ) as usize;
    for (index, frame) in frames[..captured].iter().enumerate() {
        let _ = writeln!(s, "CRASH frame {index}: {}", where_is(*frame as usize));
    }

    // The unwinder above can lose the trail. Look for candidate return addresses on the faulting stack.
    let mut hits = 0;
    let mut checked_page = 0usize;
    let mut page_readable = false;
    for index in 0..STACK_SCAN_SLOTS {
        let slot = ctx.Rsp as usize + index * std::mem::size_of::<usize>();
        let page = slot / PAGE_SIZE;
        if page != checked_page {
            checked_page = page;
            page_readable = readable(slot);
        }
        if !page_readable || hits >= MAX_STACK_HITS {
            continue;
        }
        let value = (slot as *const usize).read_unaligned();
        if !(0x1_0000..=0x7fff_ffff_ffff).contains(&value) {
            continue;
        }
        let place = where_is(value);
        if !place.ends_with("(no module)")
            && !place.starts_with("ntdll")
            && !place.starts_with("KERNEL")
        {
            let _ = writeln!(s, "CRASH stack[{index}] = {place}");
            hits += 1;
        }
    }
    emit(&s, true);
}

const FATAL: [u32; 9] = [
    0xC000_0005, // access violation
    0xC000_0006, // in-page error
    0xC000_001D, // illegal instruction
    0xC000_0094, // integer divide by zero
    0xC000_0096, // privileged instruction
    0xC000_00FD, // stack overflow
    0xC000_0374, // heap corruption
    0xC000_0409, // fast fail
    0xC000_008C, // array bounds
];
static VEH_SEEN: AtomicU32 = AtomicU32::new(0);
static BUSY: AtomicBool = AtomicBool::new(false);
static PREVIOUS_FILTER: AtomicUsize = AtomicUsize::new(0);

/// Sees every exception first. Reports the fatal kinds (the first few only: the game may catch some itself) and
/// always lets the search go on (0 = EXCEPTION_CONTINUE_SEARCH).
unsafe extern "system" fn vectored_handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    let code = (*(*info).ExceptionRecord).ExceptionCode as u32;
    if !FATAL.contains(&code)
        || VEH_SEEN.fetch_add(1, Ordering::Relaxed) >= 6
        || BUSY.swap(true, Ordering::Relaxed)
    {
        return 0;
    }
    report("first chance, the game may still handle it", info);
    BUSY.store(false, Ordering::Relaxed);
    0
}

/// Nobody handled it: the process is going down. Report, dump (debug mode), then pass it on.
unsafe extern "system" fn unhandled_filter(info: *const EXCEPTION_POINTERS) -> i32 {
    if !BUSY.swap(true, Ordering::Relaxed) {
        report("UNHANDLED, the game is dying", info);
        if debug() {
            write_dump(info);
        }
    }
    let previous = PREVIOUS_FILTER.load(Ordering::Relaxed);
    if previous != 0 {
        let previous: unsafe extern "system" fn(*const EXCEPTION_POINTERS) -> i32 =
            std::mem::transmute(previous);
        return previous(info);
    }
    0
}

unsafe fn write_dump(info: *const EXCEPTION_POINTERS) {
    use std::os::windows::io::AsRawHandle;
    let path = format!("Modloader/logs/hf_ally_crash_{}.dmp", unix_secs());
    let Ok(file) = File::create(&path) else {
        write_file(&format!("CRASH dump: cannot create {path}"), true);
        return;
    };
    let exception = MINIDUMP_EXCEPTION_INFORMATION {
        ThreadId: GetCurrentThreadId(),
        ExceptionPointers: info as *mut EXCEPTION_POINTERS,
        ClientPointers: 0,
    };
    // MiniDumpWithDataSegs (0x1) | WithIndirectlyReferencedMemory (0x40) | WithThreadInfo (0x1000)
    let ok = MiniDumpWriteDump(
        GetCurrentProcess(),
        GetCurrentProcessId(),
        file.as_raw_handle() as _,
        0x1 | 0x40 | 0x1000,
        &exception,
        std::ptr::null(),
        std::ptr::null(),
    );
    emit(
        &format!(
            "CRASH dump {path}: {}",
            if ok != 0 { "written" } else { "FAILED" }
        ),
        true,
    );
}

// --- megalog watchdog ---

fn watchdog() {
    use crate::native::{
        count, read, BATTLE_RESULT, CLOCK, CREATURES_BEGIN, CREATURES_END, RESERVE, ROSTER_BEGIN,
        ROSTER_END, SCENE_FLAGS, SCENE_SELECTOR,
    };
    let mut last = String::new();
    let mut tick = 0u64;
    loop {
        std::thread::sleep(Duration::from_millis(100));
        tick += 1;
        // only scalar globals and pointer values are read here: nothing a game thread can free under us
        let state = unsafe {
            format!(
                "sel={} fl={}{}{} result={} roster={} reserve={}",
                read::<u32>(SCENE_SELECTOR),
                read::<u8>(SCENE_FLAGS[0]),
                read::<u8>(SCENE_FLAGS[1]),
                read::<u8>(SCENE_FLAGS[2]),
                read::<i32>(BATTLE_RESULT),
                count(ROSTER_BEGIN, ROSTER_END, 0x18),
                read::<usize>(RESERVE + 8),
            )
        };
        if state != last {
            write_file(
                &format!("STATE {state} t={:.2}", unsafe { read::<f64>(CLOCK) }),
                false,
            );
            last = state;
        }
        if tick.is_multiple_of(5) {
            let heartbeat = unsafe {
                format!(
                    "HB t={:.2} {last} creatures={} hook={}",
                    read::<f64>(CLOCK),
                    count(CREATURES_BEGIN, CREATURES_END, 0x18),
                    current_hook()
                )
            };
            write_file(&heartbeat, false);
        }
        if tick.is_multiple_of(3) {
            flush_log();
        }
    }
}

/// Installs the crash capture (every build); starts the debug files when `debug_log` is on.
pub fn install() {
    START.get_or_init(Instant::now);
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        emit(
            &format!(
                "PANIC in hf_ally v{VERSION} build {BUILD}, hook running: {}: {info}",
                current_hook()
            ),
            true,
        );
        previous_hook(info);
    }));
    unsafe {
        AddVectoredExceptionHandler(1, Some(vectored_handler));
        let previous = SetUnhandledExceptionFilter(Some(unhandled_filter));
        PREVIOUS_FILTER.store(previous.map_or(0, |f| f as usize), Ordering::Relaxed);
    }
    if debug() {
        start_megalog();
    }
}

/// Sets the runtime debug switch from `logging.debug_log` before [`install`] starts the megalog.
pub fn set_debug(enabled: bool) {
    DEBUG_ON.store(enabled, Ordering::Relaxed);
}

/// Creates the megalog file and the watchdog thread (once).
fn start_megalog() {
    if MEGALOG_STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = std::fs::create_dir_all("Modloader/logs");
    let path = format!("Modloader/logs/hf_ally_megalog_{}.log", unix_secs());
    match File::create(&path) {
        Ok(file) => {
            if let Ok(mut guard) = FILE.lock() {
                *guard = Some(file);
            }
            write_file(&banner(), false);
            write_file(
                &format!("megalog file {path}, process id {}", unsafe {
                    GetCurrentProcessId()
                }),
                false,
            );
        }
        Err(error) => log::warn!("hf_ally: cannot create {path}: {error}"),
    }
    let _ = std::thread::Builder::new()
        .name("hf_ally_watchdog".into())
        .spawn(watchdog);
}

// --- fleet snapshot (runtime debug mode) ---

const CURRENT_LOCATION: usize = 0x1_43a1_1940; // pointer to the location the player is at
const PLAYER_ESCADRA: usize = 0x1_43a1_3eb0; // pointer to the player's fleet
const PLAYER_SIDE_VEC: (usize, usize) = (0x1_43a1_58e0, 0x1_43a1_58e8); // Links (0x18 bytes, fleet pointer at +8)
const PAGE_SIZE: usize = 4096;
const PROFILE_FLEETS_BEGIN: usize = 0x1f8;
const PROFILE_FLEETS_END: usize = 0x200;
const PROFILE_FLEET_LIMIT: usize = 256;
static LAST_SNAPSHOT: Mutex<String> = Mutex::new(String::new());

/// Reads a scalar `T` after checking the first and last pages of the read.
///
/// # Safety
/// Use only for non-owning, nonzero-size plain data no larger than a page. The caller must
/// establish that the bytes belong to the expected live object, are valid for `T`, and are not
/// written concurrently. A page query does not prove those conditions or hold the mapping open.
pub unsafe fn rd<T: Copy>(addr: usize) -> Option<T> {
    let end = addr.checked_add(std::mem::size_of::<T>())?;
    let last = end - 1;
    if addr < 0x1_0000 || !readable(addr) {
        return None;
    }
    // The first byte is mapped above; the last byte needs its own check only when the read spans two pages.
    let spans_two_pages = last / PAGE_SIZE != addr / PAGE_SIZE;
    if spans_two_pages && !readable(last) {
        return None;
    }
    Some((addr as *const T).read_unaligned())
}

/// Reads and bounds the profile's fleet pointer vector header without dereferencing its entries.
///
/// # Safety
/// `profile` must identify the current game-owned profile while called on the game thread.
unsafe fn profile_fleet_slots(profile: usize) -> Option<(usize, usize)> {
    let (begin, end) = (
        rd::<usize>(profile + PROFILE_FLEETS_BEGIN),
        rd::<usize>(profile + PROFILE_FLEETS_END),
    );
    let (begin, end) = (begin?, end?);
    if begin < 0x10000 || end < begin {
        return None;
    }
    let count = (end - begin) / std::mem::size_of::<usize>();
    if count > PROFILE_FLEET_LIMIT {
        return None;
    }

    Some((begin, count))
}

/// Check every page of a bounded native span without manufacturing an owning container.
///
/// # Safety
/// Readability alone is not lifetime or identity. The caller must establish native ownership,
/// keep the span alive on its owning thread, and bound `bytes` before calling. Do not cache the
/// result across native mutations, callbacks or threads.
pub unsafe fn readable_span(address: usize, bytes: usize) -> bool {
    if address < 0x1_0000 {
        return false;
    }
    let Some(end) = address.checked_add(bytes) else {
        return false;
    };
    let mut cursor = address;
    while cursor < end {
        if !readable(cursor) {
            return false;
        }
        let Some(next_page) = (cursor / PAGE_SIZE + 1).checked_mul(PAGE_SIZE) else {
            return false;
        };
        cursor = next_page;
    }
    true
}

/// One line per ship of a fleet: the creature behind each list entry with the flags a landing or a battle uses
/// (`m_incity` +0xb6c, `m_landing` +0xb6d, AI flag +0xbd4, order +0xd70, state +0xbdc, side +0xad4).
unsafe fn ships_of(fleet: usize) -> String {
    let Some(sentinel) = rd::<usize>(fleet + 0x100) else {
        return "list unreadable".into();
    };
    let mut out = String::new();
    let mut node = rd::<usize>(sentinel).unwrap_or(sentinel);
    for _ in 0..64 {
        if node == sentinel || node == 0 {
            return out;
        }
        let prop = rd::<usize>(node + 0x10).unwrap_or(0);
        // the game's own accessor (what `FUN_1400f5b40` calls on every list entry); null when the ship is not spawned
        let creature = if prop != 0 && readable(prop) {
            let linked: unsafe extern "C" fn(usize) -> usize =
                std::mem::transmute(0x1_4021_b070usize);
            linked(prop)
        } else {
            0
        };
        if creature == 0 {
            let _ = write!(out, " [prop={prop:#x} no creature]");
        } else {
            let _ = write!(
                out,
                " [c={creature:#x} incity={} landing={} ai={} order={} state={} side={}]",
                rd::<u8>(creature + 0xb6c).map_or(-1, i32::from),
                rd::<u8>(creature + 0xb6d).map_or(-1, i32::from),
                rd::<u8>(creature + offset_of!(Creature, ai_flag)).map_or(-1, i32::from),
                rd::<i32>(creature + offset_of!(Creature, order_role)).unwrap_or(-99),
                rd::<i32>(creature + offset_of!(Creature, driver_state)).unwrap_or(-99),
                rd::<f32>(creature + offset_of!(Creature, side)).unwrap_or(f32::NAN),
            );
        }
        node = rd::<usize>(node).unwrap_or(sentinel);
    }
    out.push_str(" ...(more than 64)");
    out
}

/// Logs every fleet of the profile (side marker, flags, ships and their flags) and the player-side fleet vector the
/// landing scene reads, but only when something changed since the last snapshot. Call from game-thread hooks only.
pub unsafe fn snapshot(why: std::fmt::Arguments<'_>) {
    if !debug() {
        return;
    }
    let get_profile: unsafe extern "C" fn() -> usize =
        std::mem::transmute(crate::native::GET_PROFILE);
    let profile = get_profile();
    let here = rd::<usize>(CURRENT_LOCATION).unwrap_or(0);
    let player = rd::<usize>(PLAYER_ESCADRA).unwrap_or(0);
    let mut s = format!("location={here:#x} player_fleet={player:#x}\n");
    let mut fleets = Vec::new();
    if let Some((begin, count)) = profile_fleet_slots(profile) {
        for index in 0..count {
            fleets.push(rd::<usize>(begin + index * std::mem::size_of::<usize>()).unwrap_or(0));
        }
    }
    let _ = writeln!(
        s,
        "profile fleets: {} (enemy/neutral fleets elsewhere are not listed)",
        fleets.len()
    );
    let mut omitted = 0;
    for &fleet in &fleets {
        if fleet == 0 {
            let _ = writeln!(s, "  FLEET null entry");
            continue;
        }

        // Enemy and neutral fleets elsewhere are counted but not listed: their movement would
        // invalidate the snapshot every tick.
        let remote_nonplayer_fleet = fleet != player
            && rd::<f32>(fleet + 0x128).is_some_and(|side| side <= 0.0)
            && !(here != 0 && rd::<usize>(fleet + 0x340) == Some(here));
        if remote_nonplayer_fleet {
            omitted += 1;
            continue;
        }

        let _ = writeln!(
            s,
            "  FLEET {fleet:#x}{} side={} here={} flag461={} f368={} count108={} ships:{}",
            if fleet == player { " PLAYER" } else { "" },
            rd::<f32>(fleet + 0x128).unwrap_or(f32::NAN),
            rd::<usize>(fleet + 0x340) == Some(here),
            rd::<u8>(fleet + 0x461).map_or(-1, i32::from),
            rd::<f32>(fleet + 0x368).unwrap_or(f32::NAN),
            rd::<usize>(fleet + 0x108).map_or(-1, |count| count as i64),
            ships_of(fleet)
        );
    }
    let _ = writeln!(s, "  ({omitted} enemy/neutral fleets not listed)");
    let (vb, ve) = (
        rd::<usize>(PLAYER_SIDE_VEC.0).unwrap_or(0),
        rd::<usize>(PLAYER_SIDE_VEC.1).unwrap_or(0),
    );
    let mut entries = String::new();
    if vb >= 0x10000 && ve >= vb && (ve - vb) / 0x18 <= 64 {
        for i in 0..(ve - vb) / 0x18 {
            let node = rd::<usize>(vb + i * 0x18 + 8).unwrap_or(0);
            let _ = write!(
                entries,
                " {node:#x}{}",
                if node != 0 && fleets.contains(&node) {
                    ""
                } else {
                    "(NOT in profile)"
                }
            );
        }
    }
    let _ = writeln!(
        s,
        "  PLAYER-SIDE VECTOR (0x143a158e0): {} entries:{entries}",
        if vb == 0 {
            0
        } else {
            (ve.saturating_sub(vb)) / 0x18
        }
    );
    if let Ok(mut last) = LAST_SNAPSHOT.lock() {
        if *last == s {
            return;
        }
        *last = s.clone();
    }
    write_file(&format!("SNAPSHOT [{why}]\n{}", s.trim_end()), false);
}

/// True when `ship` (a creature pointer) is one of the ships listed in a player-side fleet of the profile.
/// Used before writing to a ship that left the battle: a destroyed ship's object is gone and must not be touched.
/// Every pointer is checked for readability first.
pub unsafe fn fleet_has_ship(ship: usize) -> bool {
    let get_profile: unsafe extern "C" fn() -> usize =
        std::mem::transmute(crate::native::GET_PROFILE);
    let profile = get_profile();
    let Some((begin, count)) = profile_fleet_slots(profile) else {
        return false;
    };
    for index in 0..count {
        let Some(fleet) = rd::<usize>(begin + index * std::mem::size_of::<usize>()) else {
            continue;
        };
        if fleet == 0 || !rd::<f32>(fleet + 0x128).is_some_and(|side| side > 0.0) {
            continue;
        }
        let Some(sentinel) = rd::<usize>(fleet + 0x100) else {
            continue;
        };
        let mut node = rd::<usize>(sentinel).unwrap_or(sentinel);
        for _ in 0..64 {
            if node == sentinel || node == 0 {
                break;
            }
            let prop = rd::<usize>(node + 0x10).unwrap_or(0);
            if prop != 0 && readable(prop) {
                let linked: unsafe extern "C" fn(usize) -> usize =
                    std::mem::transmute(0x1_4021_b070usize);
                if linked(prop) == ship {
                    return true;
                }
            }
            node = rd::<usize>(node).unwrap_or(sentinel);
        }
    }
    false
}

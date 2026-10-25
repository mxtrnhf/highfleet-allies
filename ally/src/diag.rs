//! Diagnostics for tester logs.
//!
//! * Always on: the version banner, a panic hook and an exception handler that write a crash report (what
//!   crashed, in which module and at which Ghidra address, and whether a hf_ally hook was running) into the log
//!   and flush it. The modloader buffers its log file (flexi_logger `BufferAndFlush`), so without a flush the
//!   last second before a crash is lost.
//! * Feature `megalog` (the "debug build"): a second, unbuffered file `Modloader/logs/hf_ally_megalog_<time>.log`
//!   with a heartbeat, every click, every change of the game's scene state and every spawn, a watchdog that
//!   flushes the normal log every 300 ms, and a minidump (`hf_ally_crash_<time>.dmp`) on an unhandled crash.

use std::{
    ffi::c_void,
    fmt::Write as _,
    fs::File,
    io::Write as _,
    panic,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use windows_sys::Win32::System::{
    Diagnostics::Debug::{
        AddVectoredExceptionHandler, MiniDumpWriteDump, RtlCaptureStackBackTrace, SetUnhandledExceptionFilter,
        EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION,
    },
    LibraryLoader::{GetModuleFileNameW, GetModuleHandleExW},
    Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION},
    Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId},
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD: &str = env!("HF_ALLY_BUILD");
pub const DEBUG: bool = cfg!(feature = "megalog");

/// The first line a tester's log must show: version, build time and whether this is the verbose debug build.
pub fn banner() -> String {
    format!(
        "hf_ally v{VERSION} (build {BUILD} UTC){}",
        if DEBUG { " DEBUG BUILD (megalog): verbose, writes a large extra log file and a crash dump" } else { "" }
    )
}

static START: OnceLock<Instant> = OnceLock::new();
static FILE: Mutex<Option<File>> = Mutex::new(None);

fn unix_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn write_file(msg: &str, in_crash: bool) {
    let t = START.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
    let line = format!("[+{t:9.3}] {msg}\n");
    // a crash may have happened while another thread held the lock: never wait for it there
    let guard = if in_crash { FILE.try_lock().ok() } else { FILE.lock().ok() };
    if let Some(mut guard) = guard {
        if let Some(file) = guard.as_mut() {
            let _ = file.write_all(line.as_bytes()); // unbuffered: it reaches the OS even if the process dies next
        }
    }
}

/// A line for the megalog only (debug build). Use through the `crumb!` macro so the string is not built otherwise.
pub fn crumb(msg: &str) {
    if DEBUG {
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
    let (ptr, len) = (HOOK_PTR.load(Ordering::Relaxed), HOOK_LEN.load(Ordering::Relaxed));
    if ptr == 0 {
        return "none (the game's own code)".to_string();
    }
    unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(ptr as *const u8, len)).into_owned() }
}

// --- crash report ---

/// Where `addr` lives: `module.dll+0xoffset`, plus the Ghidra address for the game's own exe.
unsafe fn where_is(addr: usize) -> String {
    let mut module = std::ptr::null_mut();
    // FROM_ADDRESS (2) | UNCHANGED_REFCOUNT (1)
    if GetModuleHandleExW(3, addr as *const u16, &mut module) == 0 || module.is_null() {
        return format!("{addr:#x} (no module)");
    }
    let mut buf = [0u16; 260];
    let n = GetModuleFileNameW(module, buf.as_mut_ptr(), 260) as usize;
    let path = String::from_utf16_lossy(&buf[..n.min(260)]);
    let name = path.rsplit(['\\', '/']).next().unwrap_or("?");
    let offset = addr - module as usize;
    if module as usize == crate::IMAGE_BASE {
        format!("{name}+{offset:#x} (Ghidra {addr:#x})")
    } else {
        format!("{name}+{offset:#x}")
    }
}

unsafe fn readable(addr: usize) -> bool {
    let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
    if VirtualQuery(addr as *const c_void, &mut info, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
        return false;
    }
    // MEM_COMMIT, and not PAGE_NOACCESS (0x01) / PAGE_GUARD (0x100)
    info.State == 0x1000 && info.Protect & 0x101 == 0 && info.Protect != 0
}

fn emit(report: &str, in_crash: bool) {
    for line in report.lines() {
        write_file(line, in_crash);
    }
    log::error!("{report}");
    crate::flush_log();
}

unsafe fn report(kind: &str, info: *const EXCEPTION_POINTERS) {
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
        let _ = writeln!(s, "CRASH access violation {how} address {:#x}", rec.ExceptionInformation[1]);
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
    let mut frames = [std::ptr::null_mut::<c_void>(); 48];
    let n = RtlCaptureStackBackTrace(0, 48, frames.as_mut_ptr(), std::ptr::null_mut()) as usize;
    for (i, f) in frames[..n].iter().enumerate() {
        let _ = writeln!(s, "CRASH frame {i}: {}", where_is(*f as usize));
    }
    // raw scan of the faulting thread's stack for return addresses (the unwinder above can lose the trail)
    let mut hits = 0;
    let mut checked_page = 0usize;
    let mut page_ok = false;
    for i in 0..384usize {
        let slot = ctx.Rsp as usize + i * 8;
        if slot / 4096 != checked_page {
            checked_page = slot / 4096;
            page_ok = readable(slot);
        }
        if !page_ok || hits >= 40 {
            continue;
        }
        let value = (slot as *const usize).read_unaligned();
        if value < 0x1_0000 || value > 0x7fff_ffff_ffff {
            continue;
        }
        let place = where_is(value);
        if !place.ends_with("(no module)") && !place.starts_with("ntdll") && !place.starts_with("KERNEL") {
            let _ = writeln!(s, "CRASH stack[{i}] = {place}");
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
    if !FATAL.contains(&code) || VEH_SEEN.fetch_add(1, Ordering::Relaxed) >= 6 || BUSY.swap(true, Ordering::Relaxed) {
        return 0;
    }
    report("first chance, the game may still handle it", info);
    BUSY.store(false, Ordering::Relaxed);
    0
}

/// Nobody handled it: the process is going down. Report, dump (debug build), then pass it on.
unsafe extern "system" fn unhandled_filter(info: *const EXCEPTION_POINTERS) -> i32 {
    if !BUSY.swap(true, Ordering::Relaxed) {
        report("UNHANDLED, the game is dying", info);
        if DEBUG {
            write_dump(info);
        }
    }
    let previous = PREVIOUS_FILTER.load(Ordering::Relaxed);
    if previous != 0 {
        let previous: unsafe extern "system" fn(*const EXCEPTION_POINTERS) -> i32 = std::mem::transmute(previous);
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
    emit(&format!("CRASH dump {path}: {}", if ok != 0 { "written" } else { "FAILED" }), true);
}

// --- megalog watchdog ---

fn watchdog() {
    use crate::{count, read, BATTLE_RESULT, CLOCK, CREATURES_BEGIN, CREATURES_END, RESERVE, ROSTER_BEGIN, ROSTER_END, SCENE_FLAGS, SCENE_SELECTOR};
    let mut last = String::new();
    let mut tick = 0u64;
    loop {
        std::thread::sleep(Duration::from_millis(100));
        tick += 1;
        // only scalar globals and pointer values are read here: nothing a game thread can free under us
        let (state, heartbeat) = unsafe {
            let state = format!(
                "sel={} fl={}{}{} result={} roster={} reserve={}",
                read::<u32>(SCENE_SELECTOR),
                read::<u8>(SCENE_FLAGS[0]),
                read::<u8>(SCENE_FLAGS[1]),
                read::<u8>(SCENE_FLAGS[2]),
                read::<i32>(BATTLE_RESULT),
                count(ROSTER_BEGIN, ROSTER_END, 0x18),
                read::<usize>(RESERVE + 8),
            );
            let heartbeat = format!(
                "HB t={:.2} {state} creatures={} hook={}",
                read::<f64>(CLOCK),
                count(CREATURES_BEGIN, CREATURES_END, 0x18),
                current_hook()
            );
            (state, heartbeat)
        };
        if state != last {
            write_file(&format!("STATE {state} t={:.2}", unsafe { read::<f64>(CLOCK) }), false);
            last = state;
        }
        if tick % 5 == 0 {
            write_file(&heartbeat, false);
        }
        if tick % 3 == 0 {
            crate::flush_log();
        }
    }
}

/// Installs the crash capture (every build) and, in the debug build, the megalog file and the watchdog.
pub fn install() {
    START.get_or_init(Instant::now);
    if DEBUG {
        let _ = std::fs::create_dir_all("Modloader/logs");
        let path = format!("Modloader/logs/hf_ally_megalog_{}.log", unix_secs());
        match File::create(&path) {
            Ok(file) => {
                if let Ok(mut guard) = FILE.lock() {
                    *guard = Some(file);
                }
                write_file(&banner(), false);
                write_file(&format!("megalog file {path}, process id {}", unsafe { GetCurrentProcessId() }), false);
            }
            Err(error) => log::warn!("hf_ally: cannot create {path}: {error}"),
        }
    }
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        emit(&format!("PANIC in hf_ally v{VERSION} build {BUILD}, hook running: {}: {info}", current_hook()), true);
        previous_hook(info);
    }));
    unsafe {
        AddVectoredExceptionHandler(1, Some(vectored_handler));
        let previous = SetUnhandledExceptionFilter(Some(unhandled_filter));
        PREVIOUS_FILTER.store(previous.map_or(0, |f| f as usize), Ordering::Relaxed);
    }
    if DEBUG {
        let _ = std::thread::Builder::new().name("hf_ally_watchdog".into()).spawn(watchdog);
    }
}

// --- fleet snapshot (debug build) ---

const CURRENT_LOCATION: usize = 0x1_43a1_1940; // pointer to the location the player is at
const PLAYER_ESCADRA: usize = 0x1_43a1_3eb0; // pointer to the player's fleet
const PLAYER_SIDE_VEC: (usize, usize) = (0x1_43a1_58e0, 0x1_43a1_58e8); // Links (0x18 bytes, fleet pointer at +8)
static LAST_SNAPSHOT: Mutex<String> = Mutex::new(String::new());

/// Reads `T` at `addr` only if the page is mapped and readable (a snapshot must never fault the game).
pub unsafe fn rd<T: Copy>(addr: usize) -> Option<T> {
    let end = addr.checked_add(std::mem::size_of::<T>())?;
    if addr < 0x1_0000 || !readable(addr) || (end - 1) / 4096 != addr / 4096 && !readable(end - 1) {
        return None;
    }
    Some((addr as *const T).read_unaligned())
}

/// One line per ship of a fleet: the creature behind each list entry with the flags a landing or a battle uses
/// (`m_incity` +0xb6c, `m_landing` +0xb6d, AI flag +0xbd4, order +0xd70, state +0xbdc, side +0xad4).
unsafe fn ships_of(fleet: usize) -> String {
    let Some(sentinel) = rd::<usize>(fleet + 0x100) else { return "list unreadable".into() };
    let mut out = String::new();
    let mut node = rd::<usize>(sentinel).unwrap_or(sentinel);
    for _ in 0..64 {
        if node == sentinel || node == 0 {
            return out;
        }
        let prop = rd::<usize>(node + 0x10).unwrap_or(0);
        // the game's own accessor (what `FUN_1400f5b40` calls on every list entry); null when the ship is not spawned
        let creature = if prop != 0 && readable(prop) {
            let linked: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(0x1_4021_b070usize);
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
                rd::<u8>(creature + 0xbd4).map_or(-1, i32::from),
                rd::<i32>(creature + 0xd70).unwrap_or(-99),
                rd::<i32>(creature + 0xbdc).unwrap_or(-99),
                rd::<f32>(creature + 0xad4).unwrap_or(f32::NAN),
            );
        }
        node = rd::<usize>(node).unwrap_or(sentinel);
    }
    out.push_str(" ...(more than 64)");
    out
}

/// Logs every fleet of the profile (side marker, flags, ships and their flags) and the player-side fleet vector the
/// landing scene reads, but only when something changed since the last snapshot. Call from game-thread hooks only.
pub unsafe fn snapshot(why: &str) {
    if !DEBUG {
        return;
    }
    let get_profile: unsafe extern "C" fn() -> usize = std::mem::transmute(crate::GET_PROFILE);
    let profile = get_profile();
    let here = rd::<usize>(CURRENT_LOCATION).unwrap_or(0);
    let player = rd::<usize>(PLAYER_ESCADRA).unwrap_or(0);
    let mut s = format!("location={here:#x} player_fleet={player:#x}\n");
    let (begin, end) = (rd::<usize>(profile + 0x1f8).unwrap_or(0), rd::<usize>(profile + 0x200).unwrap_or(0));
    let mut fleets = Vec::new();
    if begin != 0 && end >= begin && (end - begin) / 8 <= 256 {
        for i in 0..(end - begin) / 8 {
            fleets.push(rd::<usize>(begin + i * 8).unwrap_or(0));
        }
    }
    let _ = writeln!(s, "profile fleets: {} (enemy/neutral fleets elsewhere are not listed)", fleets.len());
    let mut omitted = 0;
    for &f in &fleets {
        // enemy and neutral fleets are only counted: they move every tick and would flood the log
        if f != 0 && f != player && rd::<f32>(f + 0x128).map_or(false, |side| side <= 0.0) && !(here != 0 && rd::<usize>(f + 0x340) == Some(here)) {
            omitted += 1;
            continue;
        }
        if f == 0 {
            let _ = writeln!(s, "  FLEET null entry");
            continue;
        }
        let _ = writeln!(
            s,
            "  FLEET {f:#x}{}{} side={} here={} flag461={} f368={} count108={} ships:{}",
            if f == player { " PLAYER" } else { "" },
            "",
            rd::<f32>(f + 0x128).unwrap_or(f32::NAN),
            rd::<usize>(f + 0x340) == Some(here),
            rd::<u8>(f + 0x461).map_or(-1, i32::from),
            rd::<f32>(f + 0x368).unwrap_or(f32::NAN),
            rd::<usize>(f + 0x108).map_or(-1, |v| v as i64),
            ships_of(f)
        );
    }
    let _ = writeln!(s, "  ({omitted} enemy/neutral fleets not listed)");
    let (vb, ve) = (rd::<usize>(PLAYER_SIDE_VEC.0).unwrap_or(0), rd::<usize>(PLAYER_SIDE_VEC.1).unwrap_or(0));
    let mut entries = String::new();
    if vb != 0 && ve >= vb && (ve - vb) / 0x18 <= 64 {
        for i in 0..(ve - vb) / 0x18 {
            let node = rd::<usize>(vb + i * 0x18 + 8).unwrap_or(0);
            let _ = write!(entries, " {node:#x}{}", if node != 0 && fleets.contains(&node) { "" } else { "(NOT in profile)" });
        }
    }
    let _ = writeln!(s, "  PLAYER-SIDE VECTOR (0x143a158e0): {} entries:{entries}", if vb == 0 { 0 } else { (ve.saturating_sub(vb)) / 0x18 });
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
    let get_profile: unsafe extern "C" fn() -> usize = std::mem::transmute(crate::GET_PROFILE);
    let profile = get_profile();
    let (Some(begin), Some(end)) = (rd::<usize>(profile + 0x1f8), rd::<usize>(profile + 0x200)) else { return false };
    if begin == 0 || end < begin || (end - begin) / 8 > 256 {
        return false;
    }
    for i in 0..(end - begin) / 8 {
        let Some(fleet) = rd::<usize>(begin + i * 8) else { continue };
        if fleet == 0 || !rd::<f32>(fleet + 0x128).map_or(false, |side| side > 0.0) {
            continue;
        }
        let Some(sentinel) = rd::<usize>(fleet + 0x100) else { continue };
        let mut node = rd::<usize>(sentinel).unwrap_or(sentinel);
        for _ in 0..64 {
            if node == sentinel || node == 0 {
                break;
            }
            let prop = rd::<usize>(node + 0x10).unwrap_or(0);
            if prop != 0 && readable(prop) {
                let linked: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(0x1_4021_b070usize);
                if linked(prop) == ship {
                    return true;
                }
            }
            node = rd::<usize>(node).unwrap_or(sentinel);
        }
    }
    false
}

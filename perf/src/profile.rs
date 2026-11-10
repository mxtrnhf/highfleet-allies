//! Startup-only frame wrapper. Evidence and measurement limits: docs/frame-profiling.md.

use std::{
    cell::Cell,
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{sync_channel, SyncSender, TrySendError},
        OnceLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use patchy::{PatchSession, Trampoline};
use windows_sys::Win32::System::{
    Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
    Threading::GetCurrentProcessId,
};

use crate::profile_memory::{self as memory, Workload, PROFILERS};

const FRAME: usize = 0x1401a8970;
const PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10];
const QUEUE_CAPACITY: usize = 2048;
pub const COUNTER_COUNT: usize = 11;
static COUNTERS: [AtomicU64; COUNTER_COUNT] = [const { AtomicU64::new(0) }; COUNTER_COUNT];
static ENABLED: AtomicBool = AtomicBool::new(false);
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static WRITER_FAILED: AtomicBool = AtomicBool::new(false);
static SESSION: OnceLock<Session> = OnceLock::new();

// Inclusive hf_ai spans, separately reported and never added to native totals.
pub const MOD_SPANS: usize = 5;
static MOD_TICKS: [AtomicU64; MOD_SPANS] = [const { AtomicU64::new(0) }; MOD_SPANS];
static MOD_CALLS: [AtomicU64; MOD_SPANS] = [const { AtomicU64::new(0) }; MOD_SPANS];
static MOD_OBSERVED: AtomicBool = AtomicBool::new(false);

/// Versioned scalar-only ABI for already-loaded instrumented DLLs. Durations use
/// this process's QPC ticks. Unknown IDs/invalid durations are ignored.
#[no_mangle]
pub extern "C" fn hf_perf_record_span_v1(id: u32, ticks: u64) {
    if !ENABLED.load(Ordering::Relaxed) || id as usize >= MOD_SPANS {
        return;
    }
    let Some(session) = SESSION.get() else { return };
    if ticks > session.frequency as u64 * 3600 {
        return;
    }
    MOD_OBSERVED.store(true, Ordering::Relaxed);
    MOD_TICKS[id as usize].fetch_add(ticks, Ordering::Relaxed);
    MOD_CALLS[id as usize].fetch_add(1, Ordering::Relaxed);
}

pub fn count(id: usize) {
    count_by(id, 1);
}

pub fn count_by(id: usize, amount: u64) {
    if ENABLED.load(Ordering::Relaxed) {
        if let Some(counter) = COUNTERS.get(id) {
            counter.fetch_add(amount, Ordering::Relaxed);
        }
    }
}

struct Session {
    sender: SyncSender<Row>,
    frequency: i64,
}

#[derive(Clone, Copy)]
struct Row {
    sequence: u64,
    start: i64,
    interval: Option<i64>,
    call_ticks: i64,
    before_ticks: i64,
    after_ticks: i64,
    simulation_delta: Option<f64>,
    paused: Option<u8>,
    native: [Option<i64>; 8],
    workload: Workload,
    heuristic: Option<(f32, f32)>,
    counters: [u64; COUNTER_COUNT],
    mod_ticks: [u64; MOD_SPANS],
    mod_calls: [u64; MOD_SPANS],
    mod_observed: bool,
    /// Native AI spans (QPC ticks and call counts since the previous row).
    driver_ticks: u64,
    driver_calls: u64,
    scan_ticks: u64,
    scan_calls: u64,
    dropped: u64,
    quit: u8,
}

thread_local! {
    static PREVIOUS: Cell<i64> = const { Cell::new(0) };
    static SEQUENCE: Cell<u64> = const { Cell::new(0) };
}

pub fn clock() -> i64 {
    let mut value = 0;
    // SAFETY: writable scalar stack storage, no game state accessed.
    unsafe { QueryPerformanceCounter(&mut value) };
    value
}

/// Prepare but do not install the frame detour. Caller must guarantee quiescent
/// Steam 1.163 startup and permanently retain the successful patch allocation.
pub unsafe fn prepare(
    session: &mut PatchSession,
    directory: &str,
    config: &impl serde::Serialize,
) -> Result<(), String> {
    // SAFETY: bounded code bytes of the validated, fixed-base main executable.
    let found = unsafe { std::slice::from_raw_parts(FRAME as *const u8, PROLOGUE.len()) };
    if found != PROLOGUE {
        return Err("frame prologue differs; refusing profiling hook".into());
    }
    let mut frequency = 0;
    // SAFETY: writable scalar output.
    if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0
        || frequency <= 0
        || frequency > i64::MAX / 3600
    {
        return Err("invalid QPC frequency".into());
    }
    let origin = clock();
    let directory = PathBuf::from(directory);
    if !directory.is_absolute() {
        return Err("profile_output_dir must be an absolute Windows/Wine path".into());
    }
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    // SAFETY: process ID getter has no preconditions.
    let pid = unsafe { GetCurrentProcessId() };
    let path = directory.join(format!(
        "hf_perf_v{}_{timestamp}_{pid}.csv",
        env!("CARGO_PKG_VERSION")
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "schema,sequence,wall_s,loop_interval_us,frame_call_us,observer_before_us,observer_after_us,simulation_delta_s,paused,frame_native_us,pre_us,physics_us,input_us,ai_us,game_us,sound_us,video_us,bodies,physics_bodies,contacts,joints,ai_list_entries,warning_calls,query_calls,indexed_rays,fallback_rays,font_calls,kaz_scan_calls,kaz_scan_targets,kaz_prefilter_rejections,render_skip_frames,render_calls,render_false,heuristic_p25_us,heuristic_p50_us,ai_steering_us_total,ai_creature_update_us_total,ai_threat_scan_us_total,ai_tracking_us_total,ai_boost_us_total,ai_steering_calls_total,ai_creature_update_calls_total,ai_threat_scan_calls_total,ai_tracking_calls_total,ai_boost_calls_total,ai_driver_us_total,ai_scan_us_total,ai_driver_calls_total,ai_scan_calls_total,dropped_rows_total,quit").map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;
    let metadata = serde_json::json!({"schema":1,"version":env!("CARGO_PKG_VERSION"),"pid":pid,"qpc_frequency":frequency,"config":config,
        "measurement":"game-loop wall timings; NOT presented frames or Rust-only CPU time; ai_*_us_total and ai_driver/ai_scan columns are per-frame span totals",
        "ally_config": std::fs::read_to_string("Modloader/config/ally.json").ok().and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok()),
        "ai_span_coverage":{"driver":crate::ai_spans::driver_installed(),"scans":crate::ai_spans::scan_installed()},
        "source":"docs/frame-profiling.md"});
    let mut sidecar = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.with_extension("json"))
        .map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut sidecar, &metadata).map_err(|e| e.to_string())?;
    let (sender, receiver) = sync_channel::<Row>(QUEUE_CAPACITY);
    std::thread::Builder::new()
        .name("hf-perf-csv".into())
        .spawn(move || {
            let write_rows = || -> std::io::Result<()> {
                loop {
                    match receiver.recv_timeout(Duration::from_secs(1)) {
                        Ok(row) => {
                            write_row(&mut writer, row, frequency, origin)?;
                            if row.quit != 0 {
                                writer.flush()?;
                                return Ok(());
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => writer.flush()?,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            writer.flush()?;
                            return Ok(());
                        }
                    }
                    // Flush at bounded row intervals even when a continuous queue
                    // prevents recv_timeout from expiring. No disk I/O in the hook.
                    if writer.buffer().len() >= 4096 {
                        writer.flush()?;
                    }
                }
            };
            let mut write_rows = write_rows;
            if let Err(error) = write_rows() {
                WRITER_FAILED.store(true, Ordering::Relaxed);
                log::error!("hf_perf: CSV writer stopped: {error}");
            }
        })
        .map_err(|e| e.to_string())?;
    SESSION
        .set(Session { sender, frequency })
        .map_err(|_| "profiling already prepared".to_string())?;
    let mut trampoline = Trampoline::new();
    // The first 14 bytes jump to Rust; the next 10 replay two complete,
    // position-independent stack saves, followed by the native continuation.
    trampoline.absolute_jump(frame as *const () as usize);
    trampoline.bytes(&PROLOGUE);
    trampoline.absolute_jump(FRAME + PROLOGUE.len());
    // SAFETY: only complete instructions stolen; preserves original arguments,
    // Win64 stack alignment/shadow space, callee-saved registers and u8 return.
    let patch = unsafe { session.detour_trampoline(FRAME, PROLOGUE.len(), trampoline) }
        .map_err(|e| e.to_string())?;
    let address = patch
        .trampoline_address()
        .ok_or("missing frame trampoline")?;
    ORIGINAL.store(address + 14, Ordering::Release);
    log::info!(
        "hf_perf: profile CSV prepared at {}; queue {QUEUE_CAPACITY}; not presented FPS",
        path.display()
    );
    Ok(())
}

pub fn activate() {
    ENABLED.store(true, Ordering::Release);
}

unsafe extern "win64" fn frame() -> u8 {
    // SAFETY: set before permanent installation; the trampoline stays executable
    // for process lifetime. Actual native return is AL (quit flag), not void.
    let original: unsafe extern "win64" fn() -> u8 =
        unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    let Some(session) = SESSION.get() else {
        return unsafe { original() };
    };
    if WRITER_FAILED.load(Ordering::Relaxed) {
        return unsafe { original() };
    }
    let observer_start = clock();
    let sequence = SEQUENCE.with(|s| {
        let n = s.get().saturating_add(1);
        s.set(n);
        n
    });
    let previous = PREVIOUS.with(|p| p.replace(observer_start));
    // SAFETY: all snapshots/scalars are read on the native task thread. No
    // borrowed game pointers survive the call; only numeric pointer identities.
    let before = PROFILERS.map(|global| unsafe { memory::snapshot(global) });
    let simulation_before = unsafe { memory::scalar::<f64>(0x143a119c0) };
    let start = clock();
    let quit = unsafe { original() };
    let end = clock();
    let native = std::array::from_fn(|i| unsafe {
        memory::fresh_sample(PROFILERS[i], before[i], start, end, session.frequency)
    });
    let simulation_delta = simulation_before
        .zip(unsafe { memory::scalar::<f64>(0x143a119c0) })
        .map(|(before, after)| after - before)
        .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 60.0);
    let paused = unsafe { memory::scalar::<u8>(0x143a11909) }.filter(|v| *v <= 1);
    let workload = unsafe { memory::workload() };
    let heuristic = unsafe { memory::heuristic_ring() };
    let counters = std::array::from_fn(|i| COUNTERS[i].load(Ordering::Relaxed));
    let mut row = Row {
        sequence,
        start: observer_start,
        interval: (previous > 0).then_some(observer_start - previous),
        call_ticks: end - start,
        before_ticks: start - observer_start,
        after_ticks: clock() - end,
        simulation_delta,
        paused,
        native,
        workload,
        heuristic,
        counters,
        // hf_ai spans are per-frame totals: whatever the mod DLL recorded
        // during this frame's native call, swapped to zero for the next row.
        mod_ticks: std::array::from_fn(|i| MOD_TICKS[i].swap(0, Ordering::Relaxed)),
        mod_calls: std::array::from_fn(|i| MOD_CALLS[i].swap(0, Ordering::Relaxed)),
        mod_observed: MOD_OBSERVED.load(Ordering::Relaxed),
        driver_ticks: 0,
        driver_calls: 0,
        scan_ticks: 0,
        scan_calls: 0,
        dropped: DROPPED.load(Ordering::Relaxed),
        quit,
    };
    if crate::ai_spans::driver_installed() {
        let (driver_ticks, driver_calls) = crate::ai_spans::take_driver();
        row.driver_ticks = driver_ticks;
        row.driver_calls = driver_calls;
    }
    if crate::ai_spans::scan_installed() {
        let (scan_ticks, scan_calls) = crate::ai_spans::take_scan();
        row.scan_ticks = scan_ticks;
        row.scan_calls = scan_calls;
    }
    match session.sender.try_send(row) {
        Ok(()) => (),
        Err(TrySendError::Full(_)) => {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
        Err(TrySendError::Disconnected(_)) => {
            WRITER_FAILED.store(true, Ordering::Relaxed);
        }
    }
    quit
}

fn optional(writer: &mut impl Write, value: Option<impl std::fmt::Display>) -> std::io::Result<()> {
    write!(writer, ",")?;
    if let Some(value) = value {
        write!(writer, "{value}")?;
    }
    Ok(())
}

fn write_row(
    writer: &mut impl Write,
    row: Row,
    frequency: i64,
    origin: i64,
) -> std::io::Result<()> {
    let micros = |ticks: i64| ticks as f64 * 1_000_000.0 / frequency as f64;
    write!(
        writer,
        "1,{},{:.6}",
        row.sequence,
        (row.start - origin) as f64 / frequency as f64
    )?;
    optional(writer, row.interval.map(micros))?;
    for ticks in [row.call_ticks, row.before_ticks, row.after_ticks] {
        write!(writer, ",{:.3}", micros(ticks))?;
    }
    optional(writer, row.simulation_delta)?;
    optional(writer, row.paused)?;
    for value in row.native {
        optional(writer, value)?;
    }
    for value in [
        row.workload.bodies,
        row.workload.physics_bodies,
        row.workload.contacts,
        row.workload.joints,
    ] {
        optional(writer, value)?;
    }
    optional(writer, row.workload.ai_entries)?;
    for value in row.counters {
        write!(writer, ",{value}")?;
    }
    optional(writer, row.heuristic.map(|v| v.0))?;
    optional(writer, row.heuristic.map(|v| v.1))?;
    for ticks in row.mod_ticks {
        optional(writer, row.mod_observed.then_some(micros(ticks as i64)))?;
    }
    for calls in row.mod_calls {
        optional(writer, row.mod_observed.then_some(calls))?;
    }
    let driver = crate::ai_spans::driver_installed();
    let scans = crate::ai_spans::scan_installed();
    for (available, ticks) in [(driver, row.driver_ticks), (scans, row.scan_ticks)] {
        optional(writer, available.then_some(micros(ticks as i64)))?;
    }
    for (available, calls) in [(driver, row.driver_calls), (scans, row.scan_calls)] {
        optional(writer, available.then_some(calls))?;
    }
    writeln!(writer, ",{},{}", row.dropped, row.quit)
}

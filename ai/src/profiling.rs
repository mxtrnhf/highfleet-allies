//! Optional scalar-only QPC spans. Does not load a DLL or change native state.
//! See highfleet-benchmark/docs/frame-profiling.md for inclusive/nested labels.
//!
//! The sink is hf_perf's `hf_perf_record_span_v1` export. The hf_perf module
//! name is versioned (`hf_perf_v0.5.2.dll`, ...), so the candidates are read
//! once from `Modloader/mods` and each is tried with `GetModuleHandleW` until
//! one is loaded and exports the ABI. Resolution retries are bounded: without
//! an hf_perf installed the overhead stops after a few thousand attempts.

#[cfg(feature = "profiling")]
mod enabled {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use windows_sys::Win32::System::{
        LibraryLoader::{GetModuleHandleW, GetProcAddress},
        Performance::QueryPerformanceCounter,
    };

    type Sink = extern "C" fn(u32, u64);

    /// The resolved sink: 0 = not resolved yet, `usize::MAX` = given up.
    static SINK: AtomicUsize = AtomicUsize::new(0);
    /// Failed resolution attempts so far (hf_perf absent): bounded.
    static ATTEMPTS: AtomicU64 = AtomicU64::new(0);
    /// Give up after this many failed attempts.
    const MAX_ATTEMPTS: u64 = 2_000;

    /// Candidate hf_perf module names, read once from `Modloader/mods`.
    fn candidate_names() -> &'static Vec<Vec<u16>> {
        static CANDIDATES: std::sync::OnceLock<Vec<Vec<u16>>> = std::sync::OnceLock::new();
        CANDIDATES.get_or_init(|| {
            let mut names: Vec<Vec<u16>> = vec![b"hf_perf.dll\0"
                .iter()
                .map(|&byte| byte as u16)
                .collect::<Vec<u16>>()];
            if let Ok(dir) = std::fs::read_dir("Modloader/mods") {
                for entry in dir.flatten() {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if name.contains("hf_perf") && name.ends_with(".dll") {
                        let wide = name.encode_utf16().chain([0]).collect::<Vec<u16>>();
                        if !names.contains(&wide) {
                            names.push(wide);
                        }
                    }
                }
            }
            names
        })
    }

    fn resolve() -> usize {
        for name in candidate_names() {
            // SAFETY: the candidate names are NUL-terminated UTF-16 strings;
            // lookup only, no load/inject. A null return (module not loaded)
            // moves on to the next candidate.
            let module = unsafe { GetModuleHandleW(name.as_ptr()) } as usize;
            if module == 0 {
                continue;
            }
            let export = c"hf_perf_record_span_v1";
            // SAFETY: a valid module handle; the export name is NUL-terminated.
            let address = unsafe { GetProcAddress(module as _, export.as_ptr().cast()) };
            if let Some(address) = address {
                return address as usize;
            }
        }
        0
    }

    fn sink() -> Option<Sink> {
        let known = SINK.load(Ordering::Relaxed);
        if known != 0 && known != usize::MAX {
            return Some(unsafe { std::mem::transmute::<usize, Sink>(known) });
        }
        if known == usize::MAX {
            return None;
        }
        if ATTEMPTS.fetch_add(1, Ordering::Relaxed) >= MAX_ATTEMPTS {
            SINK.store(usize::MAX, Ordering::Relaxed);
            return None;
        }
        let found = resolve();
        if found == 0 {
            return None;
        }
        SINK.store(found, Ordering::Relaxed);
        log::info!("hf_ai: inclusive profiling connected to a loaded hf_perf");
        Some(unsafe { std::mem::transmute::<usize, Sink>(found) })
    }

    fn clock() -> Option<i64> {
        let mut ticks = 0;
        // SAFETY: scalar stack output, no game memory or allocation involved.
        (unsafe { QueryPerformanceCounter(&mut ticks) } != 0).then_some(ticks)
    }

    pub struct Span {
        id: u32,
        start: Option<i64>,
        sink: Option<Sink>,
    }

    impl Span {
        pub fn new(id: u32) -> Self {
            let sink = sink();
            Self { id, start: sink.and_then(|_| clock()), sink }
        }
    }

    impl Drop for Span {
        fn drop(&mut self) {
            let (Some(start), Some(sink)) = (self.start, self.sink) else { return };
            let Some(end) = clock() else { return };
            if end >= start {
                sink(self.id, (end - start) as u64);
            }
        }
    }
}

#[cfg(feature = "profiling")]
pub use enabled::Span;

#[cfg(not(feature = "profiling"))]
pub struct Span;

#[cfg(not(feature = "profiling"))]
impl Span {
    pub fn new(_id: u32) -> Self {
        Self
    }
}

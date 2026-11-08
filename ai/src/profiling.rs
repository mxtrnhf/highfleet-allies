//! Optional scalar-only QPC spans. Does not load a DLL or change native state.
//! See highfleet-benchmark/docs/frame-profiling.md for inclusive/nested labels.

#[cfg(feature = "profiling")]
mod enabled {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::{
        LibraryLoader::{GetModuleHandleA, GetProcAddress},
        Performance::QueryPerformanceCounter,
    };

    type Sink = extern "C" fn(u32, u64);
    static SINK: OnceLock<Option<Sink>> = OnceLock::new();

    fn sink() -> Option<Sink> {
        *SINK.get_or_init(|| {
            // SAFETY: NUL-terminated constant names; lookup only, no load/inject.
            // Frame callbacks occur after loader startup. hf_perf remains loaded
            // for process lifetime, as required by all permanent hook DLLs.
            unsafe {
                let module = GetModuleHandleA(c"hf_perf_v0.4.0.dll".as_ptr().cast());
                if module.is_null() { return None; }
                let address = GetProcAddress(module, c"hf_perf_record_span_v1".as_ptr().cast())?;
                // SAFETY: versioned export's exact scalar ABI is shared with
                // hf_perf v0.4.0; the module is never unloaded during play.
                let sink: Sink = std::mem::transmute(address);
                log::info!("hf_ai: inclusive profiling connected to hf_perf v0.4.0");
                Some(sink)
            }
        })
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
            if end >= start { sink(self.id, (end - start) as u64); }
        }
    }
}

#[cfg(feature = "profiling")]
pub use enabled::Span;

#[cfg(not(feature = "profiling"))]
pub struct Span;

#[cfg(not(feature = "profiling"))]
impl Span {
    pub fn new(_id: u32) -> Self { Self }
}

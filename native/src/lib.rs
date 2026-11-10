//! Startup-only native-engine acceleration for HighFleet Steam 1.163.
//!
//! All game-owned state stays on its native thread. The diagnostic thread
//! observes only the modules' atomic counters, never native object pointers.

mod call_hook;
mod config;
mod contacts;
mod host_queries;
mod layout;
mod links;
mod media;
mod relink;
mod topology;

use std::{
    ffi::{c_char, CStr},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use config::Config;
use patchy::PatchSession;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;

highfleet_mod_api::export_logger!();

const IMAGE_BASE: usize = 0x1_4000_0000;
static SUPPORTED: AtomicBool = AtomicBool::new(false);
static INITIALIZED: AtomicBool = AtomicBool::new(false);

#[no_mangle]
unsafe extern "C" fn version(version: *const c_char) -> bool {
    if version.is_null() {
        return false;
    }
    // SAFETY: Modloader supplies a valid borrowed, NUL-terminated version
    // string for the duration of discovery. No allocation ownership is taken.
    let supported = unsafe { CStr::from_ptr(version) }.to_bytes() == b"Steam 1.163";
    SUPPORTED.store(supported, Ordering::Relaxed);
    supported
}

#[no_mangle]
unsafe extern "C" fn init() -> bool {
    log::info!(
        "hf_native v{}: experimental native engine acceleration",
        env!("CARGO_PKG_VERSION")
    );
    if !SUPPORTED.load(Ordering::Relaxed) {
        log::error!("hf_native: Steam 1.163 required; no patches installed");
        return false;
    }
    // SAFETY: A null module name obtains the current process executable's base.
    if unsafe { GetModuleHandleA(std::ptr::null()) } as usize != IMAGE_BASE {
        log::error!("hf_native: unexpected executable base; no patches installed");
        return false;
    }
    if INITIALIZED.swap(true, Ordering::Relaxed) {
        log::error!("hf_native: repeated initialization refused");
        return false;
    }
    let config = match Config::read() {
        Ok(config) => config,
        Err(error) => {
            log::error!("hf_native: {error}; no patches installed");
            return false;
        }
    };
    if !config.enabled {
        log::info!(
            "hf_native: disabled in {}; native engine unchanged",
            config::PATH
        );
        return true;
    }

    // SAFETY: Modloader initialization is the existing quiescent startup
    // installation boundary. Version/base are checked above. The modules
    // validate their native byte contracts before preparing permanent patches.
    if let Err(error) = unsafe { install(&config) } {
        log::error!("hf_native: not installed: {error}");
        return false;
    }
    if config.host_query_probe {
        // SAFETY: Only aligned imports in pinned, permanently retained modules
        // are atomically observed. Missing modules remain explicitly uncovered.
        unsafe { host_queries::install() };
    }
    log::info!(
        "hf_native: installed: link_search={} link_index={} tail_probe={} tail_probe_remove={} parts_direct_append={} contact_pool={} capacity={} relink_probe={} media_index={} media_probe={} host_query_probe={}; panels/AI cadence unchanged; in-game benefit unmeasured",
        config.link_search, config.link_index, config.tail_probe, config.tail_probe_remove, config.parts_direct_append, config.contact_pool, config.contact_pool_capacity, config.scene_relink_probe, config.media_lookup_index, config.media_lookup_probe, config.host_query_probe
    );
    start_diagnostics(config.diagnostics_interval_seconds);
    true
}

/// Prepare one batch; a pre-install error drops the session without patching.
///
/// # Safety
/// Must run once during quiescent Modloader initialization with the supported
/// fixed-base executable mapped. The DLL must remain loaded for process lifetime.
unsafe fn install(config: &Config) -> Result<(), String> {
    let mut session = PatchSession::new();
    if config.link_search {
        // SAFETY: Caller provides the validated startup/version/base contract.
        unsafe {
            links::prepare(
                &mut session,
                config.tail_probe,
                config.tail_probe_remove,
                config.link_index,
            )?
        };
    }
    if config.contact_pool {
        // SAFETY: Same startup boundary; contacts validates every native site.
        unsafe { contacts::prepare(&mut session, config.contact_pool_capacity)? };
    }
    if config.parts_direct_append {
        // SAFETY: Same startup/version/base contract; topology validates its
        // collector branch and preserves native exception/growth call locations.
        unsafe { topology::prepare(&mut session, true)? };
    }
    if config.media_lookup_index || config.media_lookup_probe {
        // SAFETY: Media validates every CALL/native body and its lifetime guard.
        // Only published singleton animation descriptors are indexed; misses and
        // unsupported cases retain the untouched native lookup.
        unsafe {
            media::prepare(
                &mut session,
                config.media_lookup_index,
                config.media_lookup_probe,
            )?
        };
    }
    if config.scene_relink_probe {
        // SAFETY: Relink validates both pass brackets and all three external
        // queries; native recursion and every returned Node remain untouched.
        unsafe { relink::prepare(&mut session)? };
    }
    session
        .install_permanently()
        .map_err(|error| error.to_string())
}

fn start_diagnostics(seconds: u64) {
    if seconds == 0 {
        return;
    }
    let result = std::thread::Builder::new()
        .name("hf_native-diagnostics".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(seconds));
            let links = links::snapshot();
            let parts = topology::snapshot();
            let contacts = contacts::snapshot();
            log::info!(
                "hf_native: link [remove={:?} unlink={:?} slink={:?} index add={} ins={} hit={} miss={} mismatch={} evict={}]; parts [direct={} growth={} fallback={} anomalies={} disabled={}]; contacts [active={} cap={} pooled=({}/{}, {}B) hits=({}/{}) native=({}/{}) cached=({}/{}) fwd=({}/{}) new={} free={} busy={} inactive={}]",
                (
                    links.remove_node_link.calls,
                    links.remove_node_link.search_hops,
                    links.remove_node_link.tail_hits,
                    links.remove_node_link.fallbacks,
                    links.remove_node_link.indexed,
                ),
                (
                    links.link_unlink_from_node.calls,
                    links.link_unlink_from_node.search_hops,
                    links.link_unlink_from_node.tail_hits,
                    links.link_unlink_from_node.fallbacks,
                    links.link_unlink_from_node.indexed,
                ),
                (
                    links.slink_destructor.calls,
                    links.slink_destructor.search_hops,
                    links.slink_destructor.tail_hits,
                    links.slink_destructor.fallbacks,
                    links.slink_destructor.indexed,
                ),
                links.add_calls,
                links.handles_inserted,
                links.index_hits,
                links.index_misses,
                links.index_mismatches,
                links.table_evictions,
                parts.direct_appends,
                parts.growth_appends,
                parts.fallback_appends,
                parts.anomalies,
                parts.disabled,
                contacts.active,
                contacts.capacity,
                contacts.records_pooled,
                contacts.cells_pooled,
                contacts.pooled_bytes,
                contacts.record_hits,
                contacts.cell_hits,
                contacts.record_native,
                contacts.cell_native,
                contacts.record_cached,
                contacts.cell_cached,
                contacts.record_forwarded,
                contacts.cell_forwarded,
                contacts.native_new_calls,
                contacts.native_free_calls,
                contacts.lock_busy,
                contacts.inactive,
            );

            let relink = relink::snapshot();
            log::info!(
                "hf_native: relink [rebuild=(load {}, level {}, complete {}, nested {}) us=(total {}, max {}) queries={} outside={} hits={} misses={} samples={} sample_nodes={} max_nodes={} complete={} incomplete={} mismatch={} contended={} sampled_roots=(distinct {}, repeat {}, full {})]; shapes={:?} shapes_outside={:?} vtables={:?} other={} vtables_outside={:?} other_outside={}; sites={:?}; shadow sample 1/256, native result authoritative",
                relink.rebuild_passes_load,
                relink.rebuild_passes_level,
                relink.rebuilds_completed,
                relink.nested_rebuilds,
                relink.rebuild_total_us,
                relink.rebuild_max_us,
                relink.queries,
                relink.queries_outside_rebuild,
                relink.hits,
                relink.misses,
                relink.samples,
                relink.sample_nodes,
                relink.sample_max_nodes,
                relink.sample_complete,
                relink.sample_incomplete,
                relink.sample_mismatches,
                relink.sample_contended,
                relink.sample_distinct_roots,
                relink.sample_repeat_roots,
                relink.sample_root_table_full,
                relink.root_shapes,
                relink.root_shapes_outside,
                relink.root_vtables,
                relink.root_vtable_other,
                relink.root_vtables_outside,
                relink.root_vtable_other_outside,
                relink.sites,
            );

            let queries = host_queries::snapshot();
            log::info!(
                "hf_native: host_queries [active={:?} calls={:?}] order=(ai1.1.0, ally1.0.4, mc0.1.0); VirtualQuery unchanged; QoL/native/Wine internal calls not covered",
                queries.active,
                queries.calls,
            );

            let media = media::snapshot();
            log::info!(
                "hf_native: media_index {:?}; sites order=(animation, map_a, map_b)",
                media.index,
            );
            if media.probe.active {
                log::info!("hf_native: media_probe {:?}; observer only", media.probe);
            }
        });
    if let Err(error) = result {
        // Diagnostics are optional; their failure must not affect native state
        // or pretend that already-installed permanent patches were removed.
        log::warn!("hf_native: diagnostics thread unavailable: {error}");
    }
}

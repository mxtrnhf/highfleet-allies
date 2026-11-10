//! Startup-only native-engine acceleration for HighFleet Steam 1.163.
//!
//! All game-owned state stays on its native thread. The diagnostic thread
//! observes only the modules' atomic counters, never native object pointers.

mod call_hook;
mod config;
mod contacts;
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
    log::info!(
        "hf_native: installed: link_search={} link_index={} tail_probe={} tail_probe_remove={} parts_direct_append={} contact_pool={} capacity={} relink_probe={} media_probe={}; panels/AI cadence unchanged; in-game benefit unmeasured",
        config.link_search, config.link_index, config.tail_probe, config.tail_probe_remove, config.parts_direct_append, config.contact_pool, config.contact_pool_capacity, config.scene_relink_probe, config.media_lookup_probe
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
    if config.media_lookup_probe {
        // SAFETY: Media validates every CALL and leaves the native lookup itself
        // untouched; the native result remains authoritative on every call.
        unsafe { media::prepare(&mut session)? };
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
                "hf_native: relink [passes=(load {}, level {}, complete {}, nested {}) pass_us=(total {}, max {}) queries={} outside={} visits={} max_visits={} roots={} switches={} root_full={} census=(nodes {}, max {}) incomplete={} mismatch={}]; sites [link={:?} joint_master={:?} joint_a={:?}] (calls, shadow_visits); observer only, native result authoritative",
                relink.passes_load,
                relink.passes_level,
                relink.passes_completed,
                relink.nested_passes,
                relink.pass_total_us,
                relink.pass_max_us,
                relink.queries,
                relink.queries_outside_pass,
                relink.visits,
                relink.max_query_visits,
                relink.distinct_roots,
                relink.root_switches,
                relink.root_table_full,
                relink.census_nodes,
                relink.max_census_nodes,
                relink.shadow_incomplete,
                relink.shadow_mismatches,
                (relink.sites[0].calls, relink.sites[0].visits),
                (relink.sites[1].calls, relink.sites[1].visits),
                (relink.sites[2].calls, relink.sites[2].visits),
            );

            let media = media::snapshot();
            log::info!(
                "hf_native: media [calls={} hits={} misses={} hops={} bytes={} max_hops={} capped={} mismatch={} distinct={} hash_full={} hash_capped={}]; sites [animation={:?} type11={:?} sound={:?} font={:?} texture={:?} map_a={:?} map_b={:?}] (calls, shadow_hops)",
                media.calls,
                media.hits,
                media.misses,
                media.hops,
                media.compare_bytes,
                media.max_hops,
                media.capped,
                media.mismatches,
                media.distinct_names,
                media.name_hash_full,
                media.name_hash_capped,
                (media.sites[0].calls, media.sites[0].hops),
                (media.sites[1].calls, media.sites[1].hops),
                (media.sites[2].calls, media.sites[2].hops),
                (media.sites[3].calls, media.sites[3].hops),
                (media.sites[4].calls, media.sites[4].hops),
                (media.sites[5].calls, media.sites[5].hops),
                (media.sites[6].calls, media.sites[6].hops),
            );
        });
    if let Err(error) = result {
        // Diagnostics are optional; their failure must not affect native state
        // or pretend that already-installed permanent patches were removed.
        log::warn!("hf_native: diagnostics thread unavailable: {error}");
    }
}

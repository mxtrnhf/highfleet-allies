//! Missile Defense results: native replay volumes and frozen vanilla combat panels.

use std::{
    cell::Cell,
    ffi::CString,
    mem::{offset_of, size_of},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use patchy::{PatchSession, ReturnType, Trampoline};

use crate::{
    ally_panels::{
        self, PanelImage, GET_PLAYER_CREATURE, PANEL_HEIGHT, PANEL_SUPPRESSED, PANEL_WIDTH,
    },
    diag,
    native::{live_ship, read, ROSTER_BEGIN},
    native_layout::{Body, Creature, Node, NODE_CODE, NODE_OWNER},
};

use super::{
    design,
    missile_defense::{self, vector_span, CUSTOM_DEFENDER_LIMIT},
    ControlGeometry, EDITOR_MODE, LINK_NODE_STRIDE, LINK_TARGET,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    Editor,
    Ally(usize),
    Enemy(usize),
}

pub(super) struct Snapshot(CString);

impl Snapshot {
    pub(super) fn path(&self) -> &std::ffi::CStr {
        &self.0
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        if let Ok(path) = self.0.to_str() {
            if let Err(error) = std::fs::remove_file(path) {
                log::warn!("hf_ally: cannot remove damage snapshot {path}: {error}");
            }
        }
    }
}

#[derive(Clone)]
pub(super) enum Replay {
    Saved(Arc<Snapshot>),
    Destroyed,
}

struct CombatShip {
    slot: Slot,
    address: usize,
    id: i64,
    name: CString,
    image: Option<PanelImage>,
}

struct Encounter {
    preserve: bool,
    ships: Vec<CombatShip>,
    already_destroyed: Vec<(Slot, Replay)>,
    bank: usize,
    cursor: usize,
    render_frames: u32,
    hidden_hud_frames: u32,
    suppressed_frames: u32,
}

#[derive(Clone)]
pub(super) struct Page {
    pub(super) name: CString,
    pub(super) destroyed: bool,
    image: Option<PanelImage>,
}

impl Page {
    pub(super) fn has_panel(&self) -> bool {
        self.image.is_some()
    }
}

#[derive(Default)]
struct Report {
    pages: Vec<Page>,
    replay: Vec<(Slot, Replay)>,
    bank: usize,
}

#[derive(Clone, Copy)]
struct Preview {
    area: ControlGeometry,
    image: PanelImage,
}

static ENCOUNTER: Mutex<Option<Encounter>> = Mutex::new(None);
static REPORT: Mutex<Option<Report>> = Mutex::new(None);
static PREVIEW: Mutex<Option<Preview>> = Mutex::new(None);
static SERIAL: AtomicUsize = AtomicUsize::new(0);

pub(super) fn pages(index: usize) -> (usize, Option<Page>) {
    REPORT
        .try_lock()
        .ok()
        .and_then(|report| {
            let report = report.as_ref()?;
            Some((report.pages.len(), report.pages.get(index).cloned()))
        })
        .unwrap_or_default()
}

pub(super) fn replay() -> Vec<(Slot, Replay)> {
    REPORT
        .try_lock()
        .ok()
        .and_then(|report| Some(report.as_ref()?.replay.clone()))
        .unwrap_or_default()
}

pub(super) fn invalidate_replay() {
    let replay = REPORT
        .try_lock()
        .ok()
        .and_then(|mut report| Some(std::mem::take(&mut report.as_mut()?.replay)));
    drop(replay);
}

pub(super) fn clear() {
    if let Ok(mut report) = REPORT.try_lock() {
        let old = report.take();
        drop(report);
        drop(old);
    }
    preview(None);
}

pub(super) fn discard_encounter() {
    if let Ok(mut encounter) = ENCOUNTER.try_lock() {
        *encounter = None;
    }
}

pub(super) fn preview(page: Option<(ControlGeometry, Page)>) {
    if let Ok(mut preview) = PREVIEW.try_lock() {
        *preview = page.and_then(|(area, page)| {
            Some(Preview {
                area,
                image: page.image?,
            })
        });
    }
}

/// Called after the native spawner has populated its combat Link vector. The
/// spawner moves the template's existing bodies; it does not clone Creature.
pub(super) unsafe fn begin(
    roster: &[usize],
    staged: &[(usize, Slot)],
    preserve: bool,
    mut previous: Vec<(Slot, Replay)>,
) {
    let mut ships = Vec::new();
    for &ship in roster {
        let Some(&(_, slot)) = staged.iter().find(|&&(address, _)| address == ship) else {
            continue;
        };
        if !diag::readable_span(ship, size_of::<Creature>())
            || !live_ship(ship)
            || ships.iter().any(|entry: &CombatShip| entry.address == ship)
        {
            continue;
        }
        let name = design::display_name(ship);
        if let Some((parts, damaged, health, health_max)) = design::health_summary(ship) {
            log::info!(
                "hf_ally: spawned {} as {:?} with {damaged}/{parts} parts below max health, hull {health:.0}/{health_max:.0}",
                name.to_string_lossy(),
                slot
            );
        }
        ships.push(CombatShip {
            slot,
            address: ship,
            id: read(ship + offset_of!(Node, id)),
            name,
            image: None,
        });
    }
    log::info!(
        "hf_ally: recording damage for {} missile defenders",
        ships.len()
    );
    previous.retain(|(_, state)| matches!(state, Replay::Destroyed));
    // Keep the last completed report intact if a new launch aborts. Each bank
    // is reused, so repeatedly launching never grows the render-target pool.
    let bank = REPORT
        .try_lock()
        .ok()
        .and_then(|report| Some(1 - report.as_ref()?.bank))
        .unwrap_or(0);
    if let Ok(mut encounter) = ENCOUNTER.try_lock() {
        *encounter = Some(Encounter {
            preserve,
            ships,
            already_destroyed: previous,
            bank,
            cursor: 0,
            render_frames: 0,
            hidden_hud_frames: 0,
            suppressed_frames: 0,
        });
    }
}

/// Capture before native teardown clears the roster and moves ships to map
/// properties. Only current roster members with the original Node ID are read.
pub(super) unsafe fn finish(roster: &[usize]) {
    let Some(encounter) = ENCOUNTER
        .try_lock()
        .ok()
        .and_then(|mut active| active.take())
    else {
        return;
    };
    if encounter.ships.is_empty() {
        return;
    }
    let mut report = Report {
        replay: encounter.already_destroyed,
        bank: encounter.bank,
        ..Report::default()
    };
    for CombatShip {
        slot,
        address,
        id,
        name,
        image,
    } in encounter.ships
    {
        let alive = roster.contains(&address) && live_identity(address, id);
        if encounter.preserve {
            if alive {
                if let Some((parts, damaged, health, health_max)) = design::health_summary(address)
                {
                    log::info!(
                        "hf_ally: capturing {} with {damaged}/{parts} parts below max health, hull {health:.0}/{health_max:.0}",
                        name.to_string_lossy()
                    );
                }
                let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
                match design::snapshot_damage(address, &name, serial) {
                    Ok(path) => report
                        .replay
                        .push((slot, Replay::Saved(Arc::new(Snapshot(path))))),
                    Err(reason) => log::warn!(
                        "hf_ally: could not preserve damage for {}: {reason}",
                        name.to_string_lossy()
                    ),
                }
            } else {
                report.replay.push((slot, Replay::Destroyed));
            }
        }
        report.pages.push(Page {
            name,
            destroyed: !alive,
            image,
        });
    }
    log::info!(
        "hf_ally: captured {} missile damage pages ({} native panels), {} saved ships; render frames: {} ({} HUD hidden, {} panel suppressed)",
        report.pages.len(),
        report.pages.iter().filter(|page| page.has_panel()).count(),
        report
            .replay
            .iter()
            .filter(|(_, state)| matches!(state, Replay::Saved(_)))
            .count(),
        encounter.render_frames,
        encounter.hidden_hud_frames,
        encounter.suppressed_frames,
    );
    if let Ok(mut latest) = REPORT.try_lock() {
        let old = latest.replace(report);
        drop(latest);
        drop(old);
    }
}

/// Caller has located `ship` in the current combat roster on the game thread.
unsafe fn live_identity(ship: usize, id: i64) -> bool {
    diag::readable_span(ship, size_of::<Creature>())
        && live_ship(ship)
        && read::<i64>(ship + offset_of!(Node, id)) == id
}

// Two banks keep the published report separate from the encounter in progress.
// The depth surface is cleared by every DrawBattleShipPanel call and is never
// composited, so one depth surface serves both banks.
struct Targets {
    images: [[Option<PanelImage>; CUSTOM_DEFENDER_LIMIT]; 2],
    depth: Option<usize>,
    failed: bool,
}

static TARGETS: Mutex<Targets> = Mutex::new(Targets {
    images: [[None; CUSTOM_DEFENDER_LIMIT]; 2],
    depth: None,
    failed: false,
});

unsafe fn target(bank: usize, index: usize) -> Option<(PanelImage, usize)> {
    let (image, depth) = {
        let targets = TARGETS.try_lock().ok()?;
        if targets.failed {
            return None;
        }
        (*targets.images.get(bank)?.get(index)?, targets.depth)
    };
    if let (Some(image), Some(depth)) = (image, depth) {
        return Some((image, depth));
    }
    // Only the render thread reaches the factory. Do not hold a mod lock
    // across native allocations or calls into the rendering engine.
    let depth = depth.or_else(|| ally_panels::create_panel_depth());
    let image = depth.and_then(|_| image.or_else(|| PanelImage::create()));
    let mut targets = TARGETS.try_lock().ok()?;
    targets.depth = depth;
    if let (Some(image), Some(depth)) = (image, depth) {
        *targets.images.get_mut(bank)?.get_mut(index)? = Some(image);
        Some((image, depth))
    } else {
        targets.failed = true;
        drop(targets);
        log::warn!("hf_ally: could not allocate Missile Defense damage panel targets");
        None
    }
}

// DrawBattleShipPanel selects its own ship through this one call before
// applying the mode-3 target lookup. Use mode 2 and substitute just the return
// value during our synchronous draw; no profile or target links are changed.
thread_local! {
    static CAPTURE_SHIP: Cell<usize> = const { Cell::new(0) };
}

unsafe extern "C" fn select_panel_ship(profile: usize, slot: u32) -> usize {
    let ship = CAPTURE_SHIP.try_with(Cell::get).unwrap_or(0);
    if ship != 0 {
        return if slot == 0 { ship } else { 0 };
    }
    let original: unsafe extern "C" fn(usize, u32) -> usize =
        std::mem::transmute(GET_PLAYER_CREATURE);
    original(profile, slot)
}

/// Replay the stock HUD branch except when an offscreen capture needs the
/// native panel pass to follow it. The HUD composite, earlier in the frame,
/// still uses the real HUD flag; spectator mode does not gain a live HUD.
unsafe extern "C" fn run_panel_pass() -> bool {
    let hud_enabled = read::<u8>(HUD_ENABLED) != 0;
    if !missile_defense::active() {
        return hud_enabled;
    }
    capture_frame(hud_enabled) || hud_enabled
}

/// Returns true if the capture touched the render targets.
unsafe fn capture_frame(hud_enabled: bool) -> bool {
    let Some((begin, count)) = vector_span(ROSTER_BEGIN, LINK_NODE_STRIDE, CUSTOM_DEFENDER_LIMIT)
    else {
        return false;
    };
    let Some((bank, index, ship, id)) = (|| {
        let mut encounter = ENCOUNTER.try_lock().ok()?;
        let encounter = encounter.as_mut()?;
        encounter.render_frames = encounter.render_frames.saturating_add(1);
        if !hud_enabled {
            encounter.hidden_hud_frames = encounter.hidden_hud_frames.saturating_add(1);
        }
        if read::<usize>(PANEL_SUPPRESSED) != 0 {
            encounter.suppressed_frames = encounter.suppressed_frames.saturating_add(1);
            return None;
        }
        let count_ships = encounter.ships.len();
        // At most one extra native panel render per frame. Dead ships retain the
        // last valid image; they are not selected even if their roster Link remains.
        for _ in 0..count_ships {
            let index = encounter.cursor;
            encounter.cursor = (index + 1) % count_ships;
            let candidate = encounter.ships.get(index)?;
            if (0..count).any(|i| {
                read::<usize>(begin + i * LINK_NODE_STRIDE + LINK_TARGET) == candidate.address
            }) && live_identity(candidate.address, candidate.id)
            {
                return Some((encounter.bank, index, candidate.address, candidate.id));
            }
        }
        None
    })() else {
        return false;
    };
    // Match the stock panel's root/owner and pod gates before touching the
    // image, so a surviving Link to a dying assembly cannot erase a snapshot.
    let root_part: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(0x1_4004_8fc0usize);
    let root = root_part(ship);
    if !diag::readable_span(root, size_of::<Body>())
        || read::<u64>(root + offset_of!(Body, meta)) == 0xd
    {
        return false;
    }
    let owner = read::<usize>(root + NODE_OWNER);
    if !diag::rd::<u8>(owner.wrapping_add(NODE_CODE)).is_some_and(|code| code & 0x2f == 0x2f) {
        return false;
    }
    let Some((image, depth)) = target(bank, index) else {
        return false;
    };
    // SAFETY: this is immediately before the stock own-panel call, on the
    // render/game thread with that frame's HUD marker table still live.
    // The current roster owns the selected Creature throughout the call.
    // The native own-panel draw that follows rebinds its own render targets.
    if CAPTURE_SHIP
        .try_with(|selected| {
            selected.set(ship);
            image.capture(depth, 2);
            selected.set(0);
        })
        .is_err()
    {
        return false;
    }
    if let Ok(mut encounter) = ENCOUNTER.try_lock() {
        if let Some(current) = encounter
            .as_mut()
            .filter(|current| current.bank == bank)
            .and_then(|current| current.ships.get_mut(index))
            .filter(|current| current.address == ship && current.id == id)
        {
            current.image = Some(image);
        }
    }
    true
}

// Steam 1.163. Replace the branch after CMP byte [HUD_ENABLED],0. The native
// render state has already been prepared at 0x140309a8d on both paths.
const HUD_ENABLED: usize = 0x1_43a1_1961;
const BATTLE_HUD_BRANCH: usize = 0x1_4030_9a9a;
const BATTLE_PANEL_PASS: usize = 0x1_4030_9aa0;
const BATTLE_SKIP_PANELS: usize = 0x1_4030_9b5b;
const PANEL_SHIP_SITE: usize = 0x1_402f_e09a;
// Editor labels have been queued into their offscreen target. Draw immediately
// before the native flush, at a distinct CALL from QoL's elevation hint.
const EDITOR_FLUSH_SITE: usize = 0x1_4030_3fc2;
const EDITOR_FLUSH: usize = 0x1_4008_3b50;

/// Install the Steam 1.163 panel capture and editor composite during serialized startup.
pub(super) unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    let mut select = Trampoline::new();
    select.preserved_call(select_panel_ship as *const (), &[], &[], ReturnType::Rax);
    select.absolute_jump(PANEL_SHIP_SITE + 5);
    session
        .detour_trampoline(PANEL_SHIP_SITE, 5, select)
        .map_err(|err| err.to_string())?;

    let mut battle = Trampoline::new();
    let skip = battle.new_label();
    battle.preserved_call(run_panel_pass as *const (), &[], &[], ReturnType::Rax);
    battle.bytes(&[0x84, 0xc0]); // TEST AL,AL
    battle.jump_if(patchy::Condition::Equal, skip);
    battle.absolute_jump(BATTLE_PANEL_PASS);
    battle.bind(skip).map_err(|err| err.to_string())?;
    battle.absolute_jump(BATTLE_SKIP_PANELS);
    session
        .detour_trampoline(
            BATTLE_HUD_BRANCH,
            BATTLE_PANEL_PASS - BATTLE_HUD_BRANCH,
            battle,
        )
        .map_err(|err| err.to_string())?;

    let mut draw = Trampoline::new();
    draw.preserved_call(render as *const (), &[], &[], ReturnType::None);
    draw.absolute_call(EDITOR_FLUSH as *const ());
    draw.absolute_jump(EDITOR_FLUSH_SITE + 5);
    session
        .detour_trampoline(EDITOR_FLUSH_SITE, 5, draw)
        .map_err(|err| err.to_string())?;
    Ok(())
}

unsafe extern "C" fn render() {
    if read::<u8>(EDITOR_MODE) == 0 {
        return;
    }
    super::airstrike::render_icons();
    let Some(preview) = PREVIEW.try_lock().ok().and_then(|preview| *preview) else {
        return;
    };
    let ControlGeometry {
        position: [x, y],
        dimensions: [width, height],
    } = preview.area;
    let scale = (width / PANEL_WIDTH).min(height / PANEL_HEIGHT);
    // SAFETY: the editor pass has bound its own label target. This draws only
    // a retained image, not the combat renderer or any dead Creature.
    preview.image.draw(
        x - PANEL_WIDTH * scale * 0.5,
        y - PANEL_HEIGHT * scale * 0.5,
        0.0,
        scale,
    );
}

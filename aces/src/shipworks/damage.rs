//! A.C.E. results: native replay volumes and frozen vanilla combat panels.

use std::{
    cell::Cell,
    ffi::{CStr, CString},
    mem::{offset_of, size_of},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use patchy_host::Condition;

use crate::patchy::{PatchSession, ReturnType, Trampoline};

use crate::{
    memory::{self, vector_span},
    native::{live_ship, read, ROSTER_BEGIN},
    native_layout::{Body, Creature, Node, NODE_CODE, NODE_OWNER},
    panel_image::{
        self, PanelImage, GET_PLAYER_CREATURE, PANEL_HEIGHT, PANEL_SUPPRESSED, PANEL_WIDTH,
    },
};

use super::{
    airstrike, design,
    missile_defense::{self, CUSTOM_DEFENDER_LIMIT},
    ControlGeometry, Entry, EDITOR_MODE, LINK_NODE_STRIDE, LINK_TARGET, SIDE_ALLY,
};

#[derive(Clone, Copy, Debug)]
pub(super) enum Scenario {
    MissileDefense,
    Airstrike,
}

const SCENARIOS: usize = 2;

impl Scenario {
    fn roster(self) -> (usize, usize) {
        match self {
            Self::MissileDefense => (ROSTER_BEGIN, CUSTOM_DEFENDER_LIMIT),
            Self::Airstrike => (airstrike::DEFENDER_ROSTER, airstrike::DEFENDER_LIMIT),
        }
    }

    /// Read the active scene's live ship Links on the game thread.
    pub(super) unsafe fn combat_roster(self) -> Option<Vec<usize>> {
        let (vector, limit) = self.roster();
        let (begin, count) = vector_span(vector, LINK_NODE_STRIDE, limit)?;
        Some(
            (0..count)
                .map(|index| read(begin + index * LINK_NODE_STRIDE + LINK_TARGET))
                .collect(),
        )
    }
}

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
                log::warn!("ACES: cannot remove damage snapshot {path}: {error}");
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
    scenario: Scenario,
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

    pub(super) fn title(&self, index: usize, count: usize) -> CString {
        let mut title = format!("{}/{}  ", index + 1, count).into_bytes();
        title.extend_from_slice(self.name.to_bytes());
        if self.destroyed {
            title.extend_from_slice(b" (DESTROYED)");
        }
        if !self.has_panel() {
            title.extend_from_slice(b" - NO PANEL CAPTURED");
        }
        CString::new(title).unwrap_or_else(|_| c"COMBAT RESULTS".into())
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
static REPORTS: Mutex<[Option<Report>; SCENARIOS]> = Mutex::new([const { None }; SCENARIOS]);
static PREVIEWS: Mutex<[Option<Preview>; SCENARIOS]> = Mutex::new([None; SCENARIOS]);
static SERIAL: AtomicUsize = AtomicUsize::new(0);

pub(super) fn pages(scenario: Scenario, index: usize) -> (usize, Option<Page>) {
    REPORTS
        .try_lock()
        .ok()
        .and_then(|reports| {
            let report = reports[scenario as usize].as_ref()?;
            Some((report.pages.len(), report.pages.get(index).cloned()))
        })
        .unwrap_or_default()
}

pub(super) fn replay(scenario: Scenario) -> Vec<(Slot, Replay)> {
    REPORTS
        .try_lock()
        .ok()
        .and_then(|reports| Some(reports[scenario as usize].as_ref()?.replay.clone()))
        .unwrap_or_default()
}

pub(super) fn invalidate_replay(scenario: Scenario) {
    let replay = REPORTS.try_lock().ok().and_then(|mut reports| {
        Some(std::mem::take(
            &mut reports[scenario as usize].as_mut()?.replay,
        ))
    });
    drop(replay);
}

pub(super) fn clear() {
    if let Ok(mut reports) = REPORTS.try_lock() {
        let old = std::mem::take(&mut *reports);
        drop(reports);
        drop(old);
    }
    if let Ok(mut previews) = PREVIEWS.try_lock() {
        previews.fill(None);
    }
}

pub(super) fn discard_encounter() {
    if let Ok(mut encounter) = ENCOUNTER.try_lock() {
        *encounter = None;
    }
}

pub(super) fn preview(scenario: Scenario, page: Option<(ControlGeometry, Page)>) {
    if let Ok(mut previews) = PREVIEWS.try_lock() {
        previews[scenario as usize] = page.and_then(|(area, page)| {
            Some(Preview {
                area,
                image: page.image?,
            })
        });
    }
}

/// Borrow the editor fleet built by FIGHT and optionally replace its one
/// original ship with a preserved volume. `None` means it was destroyed.
pub(super) unsafe fn stage_editor(
    fleet: usize,
    replay: &[(Slot, Replay)],
) -> Result<Option<usize>, &'static str> {
    let (property, original) = design::editor_defender(fleet).ok_or("invalid editor defender")?;
    let ship = match replay_for(replay, Slot::Editor) {
        Some(Replay::Destroyed) => None,
        Some(Replay::Saved(snapshot)) => match design::append(fleet, snapshot.path(), SIDE_ALLY) {
            Some(ship) => {
                log_ship_health("preserved editor ship", c"editor ship", ship);
                Some(ship)
            }
            None => {
                log::warn!("ACES: damaged editor ship could not be loaded; using editor design");
                return Ok(Some(original));
            }
        },
        None => return Ok(Some(original)),
    };
    // SAFETY: this is FIGHT's validated original Property in a scratch fleet.
    // Use the same native kill virtual as Creature_SpawnFromTemplate, not a
    // Rust destructor or a raw list edit.
    let vtable = read::<usize>(property);
    let remove: unsafe extern "C" fn(usize) = std::mem::transmute(read::<usize>(vtable + 0x48));
    remove(property);
    Ok(ship)
}

/// Append roster snapshots, taking the native damage volume instead of the
/// fresh design when available. Destroyed slots are deliberately left out.
pub(super) unsafe fn stage_entries(
    fleet: usize,
    side: f32,
    entries: &[Entry],
    slot: impl Fn(usize) -> Slot,
    replay: &[(Slot, Replay)],
    staged: &mut Vec<(usize, Slot)>,
) {
    for (index, entry) in entries.iter().enumerate() {
        let key = slot(index);
        let (ship, source) = match replay_for(replay, key) {
            Some(Replay::Destroyed) => continue,
            Some(Replay::Saved(snapshot)) => match design::append(fleet, snapshot.path(), side) {
                Some(ship) => (Some(ship), "preserved snapshot"),
                None => {
                    log::warn!(
                        "ACES: damaged snapshot for {} could not be loaded; using design",
                        entry.name.to_string_lossy()
                    );
                    (design::append(fleet, &entry.path, side), "design")
                }
            },
            None => (design::append(fleet, &entry.path, side), "design"),
        };
        if let Some(ship) = ship {
            log_ship_health(source, &entry.name, ship);
            staged.push((ship, key));
        } else {
            log::warn!(
                "ACES: could not stage defender {}",
                entry.name.to_string_lossy()
            );
        }
    }
}

fn replay_for(replay: &[(Slot, Replay)], slot: Slot) -> Option<&Replay> {
    replay
        .iter()
        .find(|(key, _)| *key == slot)
        .map(|(_, state)| state)
}

unsafe fn log_ship_health(source: &str, name: &CStr, ship: usize) {
    if let Some((parts, damaged, health, health_max)) = crate::ship_health::health_summary(ship) {
        log::info!(
            "ACES: staged {source} {}: {damaged}/{parts} parts below max health, hull {health:.0}/{health_max:.0}",
            name.to_string_lossy()
        );
    }
}

/// Called after the native spawner has populated its combat Link vector. The
/// spawner moves the template's existing bodies; it does not clone Creature.
pub(super) unsafe fn begin(
    scenario: Scenario,
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
        if !memory::readable_span(ship, size_of::<Creature>())
            || !live_ship(ship)
            || ships.iter().any(|entry: &CombatShip| entry.address == ship)
        {
            continue;
        }
        let name = design::display_name(ship);
        if let Some((parts, damaged, health, health_max)) = crate::ship_health::health_summary(ship)
        {
            log::info!(
                "ACES: spawned {} as {:?} with {damaged}/{parts} parts below max health, hull {health:.0}/{health_max:.0}",
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
        "ACES: recording {scenario:?} damage for {} defenders",
        ships.len()
    );
    previous.retain(|(_, state)| matches!(state, Replay::Destroyed));
    // Keep the last completed report intact if a new launch aborts. Each bank
    // is reused, so repeatedly launching never grows the render-target pool.
    let first_bank = scenario as usize * 2;
    let bank = REPORTS
        .try_lock()
        .ok()
        .and_then(|reports| Some(reports[scenario as usize].as_ref()?.bank ^ 1))
        .unwrap_or(first_bank);
    if let Ok(mut encounter) = ENCOUNTER.try_lock() {
        *encounter = Some(Encounter {
            scenario,
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
                if let Some((parts, damaged, health, health_max)) =
                    crate::ship_health::health_summary(address)
                {
                    log::info!(
                        "ACES: capturing {} with {damaged}/{parts} parts below max health, hull {health:.0}/{health_max:.0}",
                        name.to_string_lossy()
                    );
                }
                let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
                match design::snapshot_damage(address, &name, serial) {
                    Ok(path) => report
                        .replay
                        .push((slot, Replay::Saved(Arc::new(Snapshot(path))))),
                    Err(reason) => log::warn!(
                        "ACES: could not preserve damage for {}: {reason}",
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
        "ACES: captured {} {:?} damage pages ({} native panels), {} saved ships; render frames: {} ({} HUD hidden, {} panel suppressed)",
        report.pages.len(),
        encounter.scenario,
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
    if let Ok(mut latest) = REPORTS.try_lock() {
        let old = latest[encounter.scenario as usize].replace(report);
        drop(latest);
        drop(old);
    }
}

/// Caller has located `ship` in the current combat roster on the game thread.
unsafe fn live_identity(ship: usize, id: i64) -> bool {
    memory::readable_span(ship, size_of::<Creature>())
        && live_ship(ship)
        && read::<i64>(ship + offset_of!(Node, id)) == id
}

// Two banks per scenario separate the published report from the live encounter.
// The depth surface is cleared by every DrawBattleShipPanel call and is never
// composited, so one depth surface serves both banks.
struct Targets {
    images: [[Option<PanelImage>; CUSTOM_DEFENDER_LIMIT]; SCENARIOS * 2],
    depth: Option<usize>,
    failed: bool,
}

static TARGETS: Mutex<Targets> = Mutex::new(Targets {
    images: [[None; CUSTOM_DEFENDER_LIMIT]; SCENARIOS * 2],
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
    let depth = depth.or_else(|| panel_image::create_panel_depth());
    let image = depth.and_then(|_| image.or_else(|| PanelImage::create()));
    let mut targets = TARGETS.try_lock().ok()?;
    targets.depth = depth;
    if let (Some(image), Some(depth)) = (image, depth) {
        *targets.images.get_mut(bank)?.get_mut(index)? = Some(image);
        Some((image, depth))
    } else {
        targets.failed = true;
        drop(targets);
        log::warn!("ACES: could not allocate Shipworks damage panel targets");
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
    if !missile_defense::active() && !airstrike::active() {
        return hud_enabled;
    }
    capture_frame(hud_enabled) || hud_enabled
}

/// Returns true if the capture touched the render targets.
unsafe fn capture_frame(hud_enabled: bool) -> bool {
    let Some((bank, index, ship, id)) = (|| {
        let mut encounter = ENCOUNTER.try_lock().ok()?;
        let encounter = encounter.as_mut()?;
        let (roster, limit) = encounter.scenario.roster();
        let (begin, count) = vector_span(roster, LINK_NODE_STRIDE, limit)?;
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
    if !memory::readable_span(root, size_of::<Body>())
        || read::<u64>(root + offset_of!(Body, meta)) == 0xd
    {
        return false;
    }
    let owner = read::<usize>(root + NODE_OWNER);
    if !memory::rd::<u8>(owner.wrapping_add(NODE_CODE)).is_some_and(|code| code & 0x2f == 0x2f) {
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
    battle.jump_if(Condition::Equal, skip);
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
    airstrike::render_overlays();
    missile_defense::render_overlays();
    let Some(preview) = PREVIEWS
        .try_lock()
        .ok()
        .and_then(|previews| previews.iter().flatten().next().copied())
    else {
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

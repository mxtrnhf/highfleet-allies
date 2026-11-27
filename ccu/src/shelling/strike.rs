//! Garrison lookup and localized damage for the short shelling encounter.

use std::mem::{offset_of, size_of};

use crate::{
    memory::{rd, readable_span},
    native::{plausible, read, OFF_ACTIVE},
    native_layout::{
        Body, Creature, EscadraString, Link, Node, PartStats, SentinelList, NODE_OWNER,
    },
};

use super::{
    campaign_fleet_slots, fleet_ships, is_ship,
    presentation::{approach_offset, Point},
    ESCADRA_CLASS, ESCADRA_READ_SPAN, MAP_POSITION, NODE_CODE,
};

const ESCADRA_ROLE: usize = 0x3a8;
// Serialized as `canBeUnloaded` by Escadra_RegisterProperties at 0x14010085b.
const ESCADRA_CAN_BE_UNLOADED: usize = 0x52c;
// Creature's resolved Escadra pointer: extraction reads it directly at 0x14020f3d3.
const CREATURE_ESCADRA: usize = 0xa70;
const GARRISON_ROLE: i32 = 2;
const SIDE_SLOT: usize = 0x178;
const CITY_RADIUS: f32 = 5.0;
const MAX_GARRISON_FLEETS: usize = 16;
const MAX_PARTS: usize = 4096;
// Stock player-fired 180 mm HE (Body_AreaDamageFromBlast; see the blast notes).
const HE_EXPLOSIVE_POWER: f32 = 82.0;
const ARMOUR_HE_DIVISOR: f32 = 5.0;
// Preserve the old per-round total HP ceiling (180 + 75 + 35), not a fixed part count.
const MAX_HP_PER_IMPACT: f32 = 290.0;
const SHADOW_BINS: usize = 360;
const MODULE_CLASS: u8 = 0x0f;
const CONTAINER_CLASS_BITS: u8 = 0x30;
const ARMOUR_CATEGORY: u32 = 0x02;
// Body_Explode uses this native footprint radius to measure distance to a part's edge.
const PART_RADIUS: usize = 0x20c;
const GET_STATS: usize = 0x1_4028_1e00;
const KILL_SLOT: usize = 0x48;
const WORLD_POSITION_SLOT: usize = 0xe8;

/// Stock missile staging calls this at 0x14011471e, before building side A.
const EXPAND_FLEET_SHIPS: usize = 0x1_400f_9460;
// The expansion checks the signed f64 load time at 0x1400f9533 and replaces
// negative-time entries. The name is the design key passed at 0x1400f9579.
const SHIP_LOAD_TIME: usize = 0xc80;
const SHIP_DESIGN_NAME: usize = 0xa90;
// Ship_CloneDesignAndInit looks up the design in this library at 0x140209b3a.
const SHIP_LIBRARY: usize = 0x1_43a1_3a70;
const DESIGN_LOOKUP_SLOT: usize = 0xb8;

const _: () = {
    assert!(SHIP_LOAD_TIME + size_of::<f64>() <= size_of::<Creature>());
    assert!(SHIP_DESIGN_NAME + size_of::<EscadraString>() <= size_of::<Creature>());
    assert!(PART_RADIUS + size_of::<f32>() <= size_of::<Body>());
    assert!(CREATURE_ESCADRA + size_of::<usize>() <= size_of::<Creature>());
};

#[derive(Clone, Copy)]
struct ImpactPart {
    id: i64,
    address: usize,
    position: Point,
    radius: f32,
    armour: bool,
    health_before: f32,
    health: f32,
}

pub(super) struct DefenderDamage {
    ship_id: i64,
    ship_address: usize,
    parts: Vec<ImpactPart>,
}

pub(super) struct ImpactReport {
    pub point: Point,
    pub destroyed: usize,
    pub hull_hp_lost: f32,
}

/// Read the same compact/full discriminator as the native fleet preparer.
///
/// # Safety
/// `ship` is a checked, borrowed Creature in the current game-thread callback.
pub(super) unsafe fn is_expanded(ship: usize) -> bool {
    let load_time = read::<f64>(ship + SHIP_LOAD_TIME);
    load_time.is_finite() && load_time >= 0.0
}

/// Stock preparation owns the replacement of compact properties and their ships.
/// Do not feed its unchecked walker unresolved entries, or a missing design that
/// it could drop. Verify every fleet before any is changed, then verify the fresh
/// property lists after it runs. No ship pointer survives a native expansion.
///
/// # Safety
/// Game-thread campaign callback; `fleets` are borrowed hostile fleets from the
/// current profile and no scene transition or fleet mutation can interleave.
pub(super) unsafe fn prepare_garrisons(fleets: &[usize]) -> Result<(), &'static str> {
    let mut preparation = Vec::with_capacity(fleets.len());
    for &fleet in fleets {
        let ships = fleet_ships(fleet).ok_or("unreadable defender fleet")?;
        let mut compact = 0;
        for &ship in &ships {
            if !is_ship(ship) {
                return Err("unresolved defender property");
            }
            let load_time = read::<f64>(ship + SHIP_LOAD_TIME);
            if !load_time.is_finite() {
                return Err("invalid defender load time");
            }
            if load_time < 0.0 {
                if !has_ship_design(ship) {
                    return Err("compact defender design missing");
                }
                compact += 1;
            }
        }
        preparation.push((fleet, ships.len(), compact));
    }

    let expand: unsafe extern "C" fn(usize) = std::mem::transmute(EXPAND_FLEET_SHIPS);
    for (fleet, expected_ships, compact) in preparation {
        if compact != 0 {
            // SAFETY: every property resolved and each compact design was found.
            // Native code clones full designs, reattaches them to this fleet,
            // replaces the compact properties, and distributes their fuel.
            expand(fleet);
        }
        let ships = fleet_ships(fleet).ok_or("defenders unavailable after preparation")?;
        if ships.len() != expected_ships {
            return Err("defender count changed during preparation");
        }
        let mut part_counts = Vec::with_capacity(ships.len());
        for ship in ships {
            if !is_ship(ship) || !is_expanded(ship) {
                return Err("defender unresolved or still compact after preparation");
            }
            let count = creature_parts(ship).len();
            if count == 0 {
                return Err("defender has no assembled parts");
            }
            part_counts.push(count);
        }
        log::info!(
            "CCU: shelling defenders prepared: fleet={fleet:#x}, ships={expected_ships}, expanded={compact}, parts={part_counts:?}"
        );
    }
    Ok(())
}

/// `Ship_CloneDesignAndInit` uses the same read-only lookup before cloning.
unsafe fn has_ship_design(ship: usize) -> bool {
    let Some(name) = crate::memory::native_string(ship + SHIP_DESIGN_NAME)
        .filter(|name| !name.as_bytes().is_empty())
    else {
        return false;
    };
    let library: usize = read(SHIP_LIBRARY);
    let Some(lookup) = rd::<usize>(library)
        .and_then(|vtable| vtable.checked_add(DESIGN_LOOKUP_SLOT))
        .and_then(|slot| rd::<usize>(slot))
        .filter(|&entry| plausible(entry))
    else {
        return false;
    };
    // SAFETY: the game owns the active ship library and its returned design;
    // lookup borrows our NUL-terminated copy for this synchronous call only.
    let find: unsafe extern "C" fn(usize, *const i8) -> usize = std::mem::transmute(lookup);
    let design = find(library, name.as_ptr());
    plausible(design)
        && rd::<usize>(
            design + offset_of!(Node, children) + offset_of!(SentinelList<usize>, length),
        )
        .is_some_and(|children| children != 0 && children <= MAX_PARTS)
}

impl DefenderDamage {
    /// Copy only the identities, geometry and HP needed by the HE simulation.
    ///
    /// # Safety
    /// `ship` belongs to the initialized tactical roster on the game thread.
    pub(super) unsafe fn capture(ship: usize) -> Self {
        Self {
            ship_id: read(ship + offset_of!(Node, id)),
            ship_address: ship,
            parts: impact_parts(ship),
        }
    }

    /// Resolve one shielded HE blast entirely against the copied ship.
    pub(super) fn simulate_hit(&mut self, aim_roll: u64, direction: f32) -> Option<ImpactReport> {
        let live_parts = self.parts.iter().filter(|part| part.health > 0.0);
        let aim_index = (aim_roll as usize).checked_rem(live_parts.clone().count())?;
        let aim = live_parts.clone().nth(aim_index)?.position;
        let (surface, point) = first_contact(&self.parts, aim, direction)?;
        if !point.iter().all(|coordinate| coordinate.is_finite()) {
            return None;
        }
        let radius = HE_EXPLOSIVE_POWER.sqrt();
        let mut nearby: Vec<_> = self
            .parts
            .iter()
            .enumerate()
            .filter(|&(index, part)| index != surface && part.health > 0.0)
            .filter_map(|(index, part)| {
                let [x, y] = relative(part.position, point);
                let edge = (x.hypot(y) - part.radius).max(0.0);
                (edge < radius).then_some((edge, index))
            })
            .collect();
        nearby.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));

        let mut transmission = [1.0_f32; SHADOW_BINS];
        let mut budget = MAX_HP_PER_IMPACT;
        let mut destroyed = 0;
        let mut hull_hp_lost = 0.0;
        for (edge, index) in std::iter::once((0.0, surface)).chain(nearby) {
            if budget < 1.0 {
                break;
            }
            let Some(part) = self.parts.get_mut(index) else {
                continue;
            };
            let span = angular_span(point, *part);
            let visibility =
                span.clone().map(|bin| transmission[bin]).sum::<f32>() / span.len() as f32;
            let damage = (10.0 * radius * (1.0 - edge / radius).powi(2) * visibility).floor();
            let damage = if part.armour {
                (damage / ARMOUR_HE_DIVISOR).floor()
            } else {
                damage
            }
            .min(budget);
            if damage < 1.0 {
                continue;
            }
            let absorbed = part.health.min(damage);
            part.health -= absorbed;
            hull_hp_lost += absorbed;
            destroyed += usize::from(part.health == 0.0);
            if part.armour {
                for bin in span {
                    if let Some(energy) = transmission.get_mut(bin) {
                        *energy = (*energy - absorbed / damage).max(0.0);
                    }
                }
            }
            budget -= damage;
        }
        (hull_hp_lost > 0.0).then_some(ImpactReport {
            point,
            destroyed,
            hull_hp_lost,
        })
    }

    /// Materialize recorded HP and missing parts once, after the replay has ended.
    ///
    /// # Safety
    /// `ships` is a fresh native tactical roster outside physics traversal. Each
    /// part's cached address is only an identity and is matched against that ship's
    /// current part list before native removal.
    pub(super) unsafe fn commit(&self, ships: &[usize]) -> (usize, usize) {
        let mut changes: Vec<_> = self
            .parts
            .iter()
            .filter(|part| part.health < part.health_before)
            .collect();
        let planned = changes.len();
        if planned == 0 {
            return (0, 0);
        }
        let Some(ship) = ships.iter().copied().find(|&ship| {
            ship == self.ship_address
                && is_ship(ship)
                && crate::native::live_ship(ship)
                && read::<i64>(ship + offset_of!(Node, id)) == self.ship_id
        }) else {
            return (planned, 0);
        };
        changes.sort_unstable_by_key(|part| part.health == 0.0);
        let mut native_parts = creature_parts(ship);
        native_parts.sort_unstable();
        // Preserve the actual linked garrison before a lethal change can remove the ship.
        preserve_campaign_damage(read(ship + CREATURE_ESCADRA));
        let mut changed = 0;
        for impact in changes {
            let part = impact.address;
            if native_parts.binary_search(&part).is_err()
                || !part_is_eligible(ship, part)
                || read::<i64>(part + offset_of!(Node, id)) != impact.id
            {
                continue;
            }
            changed += usize::from(write_part_health(part, impact.health));
            if !is_ship(ship) || !crate::native::live_ship(ship) {
                break;
            }
        }
        (planned, changed)
    }
}

/// Preserve the garrison's native assembled ship records, including in the campaign save.
///
/// # Safety
/// `fleet` comes from the target Creature's game-owned Escadra Link on the raid thread. Native
/// side A owns that fleet across the encounter, including when the impacted ship is destroyed.
unsafe fn preserve_campaign_damage(fleet: usize) {
    if !plausible(fleet)
        || !readable_span(fleet, ESCADRA_CAN_BE_UNLOADED + size_of::<u8>())
        || read::<u16>(fleet + NODE_CODE) != ESCADRA_CLASS
        || read::<i32>(fleet + ESCADRA_ROLE) != GARRISON_ROLE
    {
        log::warn!("CCU: shelling damage carryover: defender has no matching garrison link");
        return;
    }
    if read::<u8>(fleet + ESCADRA_CAN_BE_UNLOADED) == 0 {
        return;
    }
    // SAFETY: fixed scalar on the verified live fleet. This is the stock opt-out for full designs.
    // UpdateStrategicFleet's unload helper (0x140111e27 / 0x140111f3e) would otherwise discard
    // the assembled ship and rebuild it from the design name; partial hull HP is not consulted.
    ((fleet + ESCADRA_CAN_BE_UNLOADED) as *mut u8).write(0);
    let id = read::<i64>(fleet + offset_of!(Node, id));
    log::info!("CCU: shelling campaign damage retained: garrison {id}, unload disabled");
}

fn relative([x, y]: Point, [ox, oy]: Point) -> Point {
    [x - ox, y - oy]
}

/// Intersect the incoming line with the parts' native bounding circles. The first intersection,
/// not the randomly selected (possibly internal) module, gets the direct hit.
fn first_contact(parts: &[ImpactPart], aim: Point, direction: f32) -> Option<(usize, Point)> {
    let [x, y] = approach_offset(direction);
    let length = x.hypot(y);
    let [nx, ny] = [x / length, y / length];
    let (part, front) = parts
        .iter()
        .enumerate()
        .filter(|&(_, part)| part.health > 0.0)
        .filter_map(|(index, part)| {
            let [dx, dy] = relative(part.position, aim);
            let lateral = dx * ny - dy * nx;
            let chord_sq = part.radius * part.radius - lateral * lateral;
            (chord_sq >= 0.0).then(|| (index, dx * nx + dy * ny + chord_sq.sqrt()))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    Some((part, [aim[0] + front * nx, aim[1] + front * ny]))
}

/// Project the bounding circle onto the blast's one-degree armour shadow table. Treat contact
/// within rounding distance of the edge as a tangent (one hemisphere), not a fully buried blast.
fn angular_span(origin: Point, part: ImpactPart) -> impl ExactSizeIterator<Item = usize> + Clone {
    let [x, y] = relative(part.position, origin);
    let distance = x.hypot(y);
    let (begin, end) = if distance + 0.01 < part.radius {
        (0, SHADOW_BINS as i32 - 1)
    } else {
        let angle = y.atan2(x).to_degrees();
        let half_span = (part.radius / distance).min(1.0).asin().to_degrees();
        (
            (angle - half_span).floor() as i32,
            (angle + half_span).ceil() as i32,
        )
    };
    (begin..end + 1).map(|degree| degree.rem_euclid(SHADOW_BINS as i32) as usize)
}

/// Current parts of a live tactical Creature, borrowed only for this synchronous callback.
unsafe fn creature_parts(ship: usize) -> Vec<usize> {
    let mut result = Vec::new();
    if !is_ship(ship) {
        return result;
    }
    let group = ship + offset_of!(Creature, parts);
    let begin: usize = read(group);
    let end: usize = read(group + size_of::<usize>());
    let Some(bytes) = end.checked_sub(begin) else {
        return result;
    };
    if bytes == 0
        || !bytes.is_multiple_of(size_of::<Link>())
        || bytes / size_of::<Link>() > MAX_PARTS
        || !plausible(begin)
        || !readable_span(begin, bytes)
    {
        return result;
    }
    for link in (begin..end).step_by(size_of::<Link>()) {
        let part: usize = read(link + offset_of!(Link, target));
        if readable_span(part, size_of::<Body>()) && read::<usize>(part + NODE_OWNER) == ship {
            result.push(part);
        }
    }
    result
}

unsafe fn impact_parts(ship: usize) -> Vec<ImpactPart> {
    creature_parts(ship)
        .into_iter()
        .filter(|&part| part_is_eligible(ship, part))
        .filter_map(|body| {
            let position = part_position(body)?;
            let radius = read::<f32>(body + PART_RADIUS);
            // Some modules have no meaningful collision radius; their centre remains targetable.
            let radius = if radius.is_finite() {
                radius.clamp(1.0, 192.0)
            } else {
                1.0
            };
            let health = read(body + offset_of!(Body, health));
            Some(ImpactPart {
                id: read(body + offset_of!(Node, id)),
                address: body,
                position,
                radius,
                armour: is_armour(body),
                health_before: health,
                health,
            })
        })
        .collect()
}

/// Real modules are eligible regardless of role, linkage or depth. Frame and Creature objects
/// are containers, not separate modules with their own blast HP.
unsafe fn part_is_eligible(ship: usize, part: usize) -> bool {
    if !readable_span(part, size_of::<Body>())
        || read::<usize>(part + NODE_OWNER) != ship
        || read::<i32>(part + OFF_ACTIVE) == 0
    {
        return false;
    }
    let class = read::<u8>(part + NODE_CODE);
    if class & (MODULE_CLASS | CONTAINER_CLASS_BITS) != MODULE_CLASS {
        return false;
    }
    let health = read::<f32>(part + offset_of!(Body, health));
    health.is_finite() && health > 0.0
}

unsafe fn part_position(part: usize) -> Option<[f32; 2]> {
    let vtable = read::<usize>(part);
    let entry =
        rd::<usize>(vtable.checked_add(WORLD_POSITION_SLOT)?).filter(|&entry| entry != 0)?;
    // SAFETY: as in Body_AreaDamageFromBlast at 0x14002ce10, the live Body's hierarchy
    // transform converts a caller-owned copy of its local shape centre to world space.
    let to_world: unsafe extern "C" fn(usize, *mut [f32; 2]) = std::mem::transmute(entry);
    let mut position: Point = read(part + offset_of!(Body, center_x));
    to_world(part, &mut position);
    position
        .iter()
        .all(|coordinate| coordinate.is_finite())
        .then_some(position)
}

unsafe fn is_armour(part: usize) -> bool {
    let get_stats: unsafe extern "C" fn(i32) -> usize = std::mem::transmute(GET_STATS);
    let stats = get_stats(read(part + offset_of!(Body, part_index)));
    rd::<u32>(stats.wrapping_add(offset_of!(PartStats, category)))
        .is_some_and(|category| category & ARMOUR_CATEGORY != 0)
}

/// Transfer the simulated final HP. Zero uses the native non-explosive removal path.
unsafe fn write_part_health(part: usize, after: f32) -> bool {
    let health_address = part + offset_of!(Body, health);
    let health = read::<f32>(health_address);
    if after >= health {
        return false;
    }
    if after == 0.0 {
        let vtable = read::<usize>(part);
        let Some(entry) = rd::<usize>(vtable.saturating_add(KILL_SLOT)).filter(|&entry| entry != 0)
        else {
            return false;
        };
        // SAFETY: this live tactical Body is removed through its own native lifecycle. The slot
        // queues dead-node collection and never transfers ownership to Rust.
        let kill: unsafe extern "C" fn(usize) = std::mem::transmute(entry);
        (health_address as *mut f32).write_unaligned(after);
        kill(part);
    } else {
        (health_address as *mut f32).write_unaligned(after);
    }
    true
}

/// Borrowed hostile garrison fleets docked at `city`. Use only in the map
/// callback that obtained them; native side links own the transition.
pub(super) unsafe fn garrison_fleets(city: [f32; 2]) -> Vec<usize> {
    let mut fleets = Vec::new();
    let Some(slots) = campaign_fleet_slots() else {
        return fleets;
    };
    for fleet in slots.step_by(size_of::<usize>()) {
        let fleet: usize = read(fleet);
        if !plausible(fleet)
            || !readable_span(
                fleet,
                ESCADRA_READ_SPAN.max(ESCADRA_ROLE + size_of::<i32>()),
            )
            || read::<u16>(fleet + NODE_CODE) != ESCADRA_CLASS
            || read::<i32>(fleet + ESCADRA_ROLE) != GARRISON_ROLE
        {
            continue;
        }
        let position: [f32; 2] = read(fleet + MAP_POSITION);
        if !position.iter().all(|coordinate| coordinate.is_finite())
            || (position[0] - city[0]).hypot(position[1] - city[1]) > CITY_RADIUS
        {
            continue;
        }
        let side = side_of(fleet);
        if !side.is_finite() || side >= 0.0 {
            continue;
        }
        if fleets.len() >= MAX_GARRISON_FLEETS {
            break;
        }
        if !fleets.contains(&fleet)
            && fleet_ships(fleet).is_some_and(|ships| ships.into_iter().any(|ship| is_ship(ship)))
        {
            fleets.push(fleet);
        }
    }
    fleets
}

unsafe fn side_of(fleet: usize) -> f32 {
    let vtable: usize = read(fleet);
    if !plausible(vtable) {
        return f32::NAN;
    }
    let Some(entry) = rd::<usize>(vtable + SIDE_SLOT).filter(|entry| *entry != 0) else {
        return f32::NAN;
    };
    let get: unsafe extern "C" fn(usize) -> f32 = std::mem::transmute(entry);
    get(fleet)
}

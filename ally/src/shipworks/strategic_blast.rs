//! Reuse the campaign's per-ship blast calculation without loading a map or profile.
//! See docs/shipworks-test-allies.md for the Steam 1.163 seams.

use std::mem::{offset_of, size_of};

use crate::{
    diag,
    native::read,
    native_layout::{Body, Creature, Node, NODE_CODE, NODE_OWNER},
};

use super::{design, write};

const INVERSE_CUBE_FALLOFF: usize = 0x1_4021_23e0;
const SORT_BLAST_PARTS: usize = 0x1_400e_2d00;
const COMPARE_FRONT_EDGE: usize = 0x1_4011_1cb0;
const SPEND_BLAST_COLUMNS: usize = 0x1_400f_d990;

// Nuke_Detonate (0x14002be2c) records each ship's own tactical blast distance
// in map units. The strategic damage routine (0x1400e6b2a) prefers that
// nonzero value over the default map-fleet distance, even inside its floor.
const BLAST_DISTANCE: usize = 0xd58;
const AI_MAP_DISTANCE_FLOOR: f32 = 1.0;
const PLAYER_MAP_DISTANCE_FLOOR: f32 = 2.0;
const AI_INTENSITY_CAP: f32 = 999.0;
const PLAYER_INTENSITY_CAP: f32 = 50.0;

// Body::mesh is an ISerializable pointer; highfleet-rs does not define the
// mesh's vertex storage. Read the same array and signed count as 0x1400e6cc2.
const MESH_VERTICES: usize = 0x48;
const MESH_VERTEX_COUNT: usize = 0x50;
const MAX_MESH_VERTICES: usize = 4096;
const MAX_BLAST_COLUMNS: usize = 32_768;
const FRAME_CODE: u8 = 0x1f;
const CREATURE_CODE: u8 = 0x2f;

// Native scratch records from 0x1400e6be8; the body is borrowed, not owned.
#[repr(C)]
struct BlastPart {
    body: usize,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
    damage: f32,
    other_damage: f32,
}

// Plain borrowed header only. 0x1400fd990 reads begin/end, spends HP in the
// array, and allocates/frees its own index list, never this storage.
#[repr(C)]
struct BlastColumns {
    begin: *mut f32,
    end: *mut f32,
    capacity_end: *mut f32,
}

const _: () = {
    assert!(offset_of!(Body, mesh) == 0x1b8);
    assert!(offset_of!(Body, health) == 0x2c8);
    assert!(offset_of!(Creature, bounds_center_x) == 0x1840);
    assert!(offset_of!(Creature, bounds_center_y) == 0x1844);
    assert!(BLAST_DISTANCE + size_of::<f32>() <= size_of::<Creature>());
    assert!(size_of::<BlastPart>() == 0x20);
    assert!(offset_of!(BlastPart, min_y) == 0x10);
    assert!(offset_of!(BlastPart, damage) == 0x18);
    assert!(size_of::<BlastColumns>() == 0x18);
};

pub(super) struct BlastResult {
    pub distance: f32,
    pub intensity: f32,
    pub hit: usize,
    pub destroyed: usize,
    pub hull_lost: f32,
}

/// Apply the native campaign blast kernel to a validated live sandbox ship.
///
/// # Safety
/// Called on the game thread immediately after `Nuke_Detonate`. The caller has
/// established a live targeted roster Creature and its owned master part.
/// No native object or projected bounds are retained between calls.
pub(super) unsafe fn apply(
    ship: usize,
    master: usize,
    origin: (f32, f32),
    yield_power: f32,
    player_side: bool,
) -> Option<BlastResult> {
    let (floor, cap) = if player_side {
        (PLAYER_MAP_DISTANCE_FLOOR, PLAYER_INTENSITY_CAP)
    } else {
        (AI_MAP_DISTANCE_FLOOR, AI_INTENSITY_CAP)
    };
    let tactical_distance = read::<f32>(ship + BLAST_DISTANCE);
    let distance = if tactical_distance.is_finite() && tactical_distance > 0.0 {
        tactical_distance
    } else {
        floor
    };
    let falloff: unsafe extern "C" fn(f32, f32, f32) -> f32 =
        std::mem::transmute(INVERSE_CUBE_FALLOFF);
    let intensity = falloff(yield_power, distance, cap);
    if !intensity.is_finite() || intensity <= 0.0 {
        return None;
    }

    // Campaign rotates the map assembly around the ship centre to face the
    // blast (0x1400e6c7a) and then rotates it back. Project the live ship's
    // existing world vertices instead; do not move its physics bodies.
    let center_x = read::<f32>(ship + offset_of!(Creature, bounds_center_x));
    let center_y = read::<f32>(ship + offset_of!(Creature, bounds_center_y));
    let dx = origin.0 - center_x;
    let dy = origin.1 - center_y;
    let length = dx.hypot(dy);
    if !length.is_finite() || length == 0.0 {
        return None;
    }
    let (source_x, source_y) = (dx / length, dy / length);
    let mut parts = Vec::new();
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    for body in design::parts(ship)? {
        let code = read::<u8>(body + NODE_CODE);
        let health = read::<f32>(body + offset_of!(Body, health));
        if code & FRAME_CODE == FRAME_CODE
            || code & CREATURE_CODE == CREATURE_CODE
            || read::<i32>(body + offset_of!(Node, state)) == 0
            || !health.is_finite()
            || health <= 0.0
        {
            continue;
        }
        if let Some(part) = project_part(body, (source_x, source_y)) {
            min_x = min_x.min(part.min_x);
            max_x = max_x.max(part.max_x);
            parts.push(part);
        }
    }
    let width = max_x - min_x;
    if !width.is_finite() || width < 0.0 || width >= MAX_BLAST_COLUMNS as f32 {
        return None;
    }
    let count = width as usize + 1;
    let mut energies = vec![intensity; count];
    let begin = energies.as_mut_ptr();
    let end = begin.add(count);
    let mut columns = BlastColumns {
        begin,
        end,
        capacity_end: end,
    };

    // SAFETY: sort only swaps the verified 0x20-byte scratch records and its
    // native comparator reads min_y. The underlying Vec does not move.
    let sort: unsafe extern "C" fn(*mut BlastPart, *mut BlastPart, usize, usize) =
        std::mem::transmute(SORT_BLAST_PARTS);
    let begin = parts.as_mut_ptr();
    sort(
        begin,
        begin.add(parts.len()),
        parts.len(),
        COMPARE_FRONT_EDGE,
    );
    let spend: unsafe extern "C" fn(*mut BlastPart, *mut BlastColumns, i32, i32, f32) =
        std::mem::transmute(SPEND_BLAST_COLUMNS);
    for part in &mut parts {
        let start = (part.min_x - min_x) as usize;
        let end = ((part.max_x - min_x) + 1.0) as usize;
        if start < end && end <= count {
            // SAFETY: the native helper only reads this live part's HP and
            // stats. All its column accesses are in [start, end); it mutates
            // our scratch columns/record synchronously and retains neither.
            spend(part, &mut columns, start as i32, end as i32, intensity);
        }
    }

    let mut result = BlastResult {
        distance,
        intensity,
        hit: 0,
        destroyed: 0,
        hull_lost: 0.0,
    };
    let mut master_damage = 0.0;
    for part in parts {
        let damage = part.damage.max(part.other_damage);
        if part.body == master {
            // The campaign removes a wrecked ship only after resolving its
            // other parts. Process the bridge last in the tactical sandbox.
            master_damage = damage;
        } else if let Some((spent, killed)) = apply_part_damage(ship, part.body, damage) {
            result.hit += 1;
            result.destroyed += usize::from(killed);
            result.hull_lost += spent;
        }
    }
    if let Some((spent, killed)) = apply_part_damage(ship, master, master_damage) {
        result.hit += 1;
        result.destroyed += usize::from(killed);
        result.hull_lost += spent;
    }
    Some(result)
}

unsafe fn project_part(body: usize, (source_x, source_y): (f32, f32)) -> Option<BlastPart> {
    let mesh = read::<usize>(body + offset_of!(Body, mesh));
    if !diag::readable_span(mesh, MESH_VERTEX_COUNT + size_of::<i32>()) {
        return None;
    }
    let count = usize::try_from(read::<i32>(mesh + MESH_VERTEX_COUNT)).ok()?;
    let vertices = read::<usize>(mesh + MESH_VERTICES);
    if !(2..=MAX_MESH_VERTICES).contains(&count)
        || !diag::readable_span(vertices, count * size_of::<[f32; 2]>())
    {
        return None;
    }
    let vtable = read::<usize>(body);
    let transform = diag::rd::<usize>(vtable.checked_add(0xe8)?).filter(|&entry| entry != 0)?;
    // SAFETY: `body` comes from the live Creature's part vector. Slot +0xe8
    // transforms a caller-owned mutable local point through the native body
    // hierarchy, touching no game-owned geometry.
    let to_world: unsafe extern "C" fn(usize, *mut [f32; 2]) = std::mem::transmute(transform);
    let mut part = BlastPart {
        body,
        min_x: f32::INFINITY,
        max_x: f32::NEG_INFINITY,
        min_y: f32::INFINITY,
        max_y: f32::NEG_INFINITY,
        damage: 0.0,
        other_damage: 0.0,
    };
    // The game starts at vertex 1 (0x1400e6cd3), not vertex 0.
    for index in 1..count {
        let mut vertex = read::<[f32; 2]>(vertices + index * size_of::<[f32; 2]>());
        to_world(body, &mut vertex);
        let [x, y] = vertex;
        let across = y * source_x - x * source_y;
        let depth = -x * source_x - y * source_y;
        if !across.is_finite() || !depth.is_finite() {
            return None;
        }
        part.min_x = part.min_x.min(across);
        part.max_x = part.max_x.max(across);
        part.min_y = part.min_y.min(depth);
        part.max_y = part.max_y.max(depth);
    }
    Some(part)
}

unsafe fn apply_part_damage(ship: usize, part: usize, damage: f32) -> Option<(f32, bool)> {
    // A previous native kill can detach another part. Revalidate its owner
    // and state before writing HP; no native value is owned or dropped here.
    if !damage.is_finite()
        || damage <= 0.0
        || !diag::readable_span(part, size_of::<Body>())
        || read::<usize>(part + NODE_OWNER) != ship
    {
        return None;
    }
    let health_address = part + offset_of!(Body, health);
    let health = read::<f32>(health_address);
    if !health.is_finite() || health <= 0.0 || read::<i32>(part + offset_of!(Node, state)) == 0 {
        return None;
    }
    let after = (health - damage).max(0.0);
    let killed = after == 0.0;
    if killed {
        let vtable = read::<usize>(part);
        let entry = diag::rd::<usize>(vtable.checked_add(0x48)?).filter(|&entry| entry != 0)?;
        // SAFETY: the live Body's native kill slot queues removal without the
        // ammo/fuel explosion path of Body_ApplyDamage. Do not use it again.
        let kill: unsafe extern "C" fn(usize) = std::mem::transmute(entry);
        write(health_address, after);
        kill(part);
    } else {
        write(health_address, after);
    }
    Some((health - after, killed))
}

//! Read-only part traversal over live game-owned ships.

use crate::{
    memory::{self, vector_span},
    native::{read, LINK_STRIDE},
    native_layout::{Body, Creature, Link, NODE_OWNER},
};
use std::mem::{offset_of, size_of};

const PART_LIMIT: usize = 4096;

/// Borrowed part identities from a live editor or combat ship, on the game thread.
pub unsafe fn parts(ship: usize) -> Option<impl Iterator<Item = usize>> {
    let (begin, count) = vector_span(ship + offset_of!(Creature, parts), LINK_STRIDE, PART_LIMIT)?;
    Some(
        (0..count)
            .map(move |index| read::<usize>(begin + index * LINK_STRIDE + offset_of!(Link, target)))
            .filter(move |&part| {
                memory::readable_span(part, size_of::<Body>())
                    && read::<usize>(part + NODE_OWNER) == ship
            }),
    )
}

/// Diagnostics for Preserve Damage: count live gameplay parts and those below max HP,
/// plus the summed current and maximum hull health. Read-only; the caller supplies a
/// Creature validated against the current roster.
pub unsafe fn health_summary(ship: usize) -> Option<(usize, usize, f32, f32)> {
    let mut total = 0usize;
    let mut damaged = 0usize;
    let mut health_total = 0.0f32;
    let mut max_total = 0.0f32;
    for part in parts(ship)? {
        let health = read::<f32>(part + offset_of!(Body, health));
        let health_max = read::<f32>(part + offset_of!(Body, health_max));
        if !health.is_finite() || !health_max.is_finite() || health_max <= 0.0 {
            continue;
        }
        total += 1;
        health_total += health;
        max_total += health_max;
        if health < health_max {
            damaged += 1;
        }
    }
    Some((total, damaged, health_total, max_total))
}

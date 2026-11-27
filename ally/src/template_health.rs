//! Spawn template health logging.

use crate::{
    memory::readable_span,
    native::{plausible, read, SHIP_LIST_LIMIT},
    native_layout::{Body, Node},
};
use std::mem::{offset_of, size_of};

/// Part count, damaged count, and current/max HP of a spawn template's remaining parts.
/// Absolute counts and HP reveal damage that the percentage alone can hide.
///
/// # Safety
/// Call on the game thread with a spawn template that is still valid.
pub(crate) unsafe fn template_health_summary(template: usize) -> Option<(usize, usize, f32, f32)> {
    if !plausible(template) || !readable_span(template, offset_of!(Node, children)) {
        return None;
    }
    let head: usize = read(template + offset_of!(Node, children));
    if !plausible(head) {
        return None;
    }
    let mut node: usize = read(head);
    let mut current = 0.0f32;
    let mut maximum = 0.0f32;
    let mut parts = 0;
    let mut damaged = 0;
    for _ in 0..SHIP_LIST_LIMIT * 64 {
        if node == head {
            break;
        }
        if !plausible(node) || !readable_span(node, 0x18) {
            return None;
        }
        let part: usize = read(node + 0x10);
        if part != 0 && readable_span(part, offset_of!(Body, health_max) + size_of::<f32>()) {
            let health: f32 = read(part + offset_of!(Body, health));
            let max: f32 = read(part + offset_of!(Body, health_max));
            if health.is_finite() && max.is_finite() && max > 0.0 {
                parts += 1;
                damaged += usize::from(health < max);
                current += health.max(0.0);
                maximum += max;
            }
        }
        node = read(node);
    }
    (node == head && maximum > 0.0).then_some((parts, damaged, current, maximum))
}

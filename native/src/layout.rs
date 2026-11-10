//! Native prefix contracts, not owning views of game memory.
//!
//! Upstream main and the consuming crates' pinned revision were both reviewed
//! at 05c62078c6f896c141713a68b3991663e121b7bf (2026-11-10). The library covers
//! Node/Link/sentinel-cell layouts; native code still owns their lifetimes.

use highfleet::general::{Link, Node, SentinelList, SentinelListNode};
use std::mem::{offset_of, size_of};

pub const NODE_LINKS: usize = offset_of!(Node, registered_links);
pub const NODE_ID: usize = offset_of!(Node, id);
pub const NODE_CHILDREN: usize = offset_of!(Node, children);
pub const LIST_HEAD: usize = offset_of!(SentinelList<*mut Link>, head);
pub const LIST_COUNT: usize = offset_of!(SentinelList<*mut Link>, length);
pub const CELL_NEXT: usize = offset_of!(SentinelListNode<*mut Link>, next);
pub const CELL_PREVIOUS: usize = offset_of!(SentinelListNode<*mut Link>, previous);
pub const CELL_LINK: usize = offset_of!(SentinelListNode<*mut Link>, value);
pub const LINK_VTABLE: usize = offset_of!(Link, vtable);
pub const LINK_TARGET: usize = offset_of!(Link, target);
pub const CELL_BYTES: usize = size_of::<SentinelListNode<*mut Link>>();
pub const NODE_CHILDREN_HEAD: usize = NODE_CHILDREN + LIST_HEAD;
pub const NODE_CHILDREN_COUNT: usize = NODE_CHILDREN + LIST_COUNT;

const _: () = {
    assert!(size_of::<Link>() == 0x18);
    assert!(NODE_LINKS == 0x50);
    assert!(NODE_ID == 0x78);
    assert!(NODE_CHILDREN == 0x100);
    assert!(NODE_CHILDREN_HEAD == 0x100);
    assert!(NODE_CHILDREN_COUNT == 0x108);
    assert!(LIST_HEAD == 0);
    assert!(LIST_COUNT == 8);
    assert!(CELL_NEXT == 0);
    assert!(CELL_PREVIOUS == 8);
    assert!(CELL_LINK == 0x10);
    assert!(LINK_VTABLE == 0);
    assert!(LINK_TARGET == 8);
    assert!(CELL_BYTES == 0x18);
};

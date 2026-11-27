//! Bounded reads of borrowed Steam 1.163 objects.

use crate::native::read;
use std::{
    ffi::{c_void, CString},
    mem::size_of,
};
use windows_sys::Win32::System::Memory::{
    VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS,
};

pub const PAGE_SIZE: usize = 4096;

pub unsafe fn readable(addr: usize) -> bool {
    let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
    if VirtualQuery(
        addr as *const c_void,
        &mut info,
        std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
    ) == 0
    {
        return false;
    }
    info.State == MEM_COMMIT
        && info.Protect & (PAGE_NOACCESS | PAGE_GUARD) == 0
        && info.Protect != 0
}

/// Read a scalar after checking its first and last pages.
///
/// # Safety
/// Use only for non-owning, nonzero-size plain data no larger than a page. The caller must
/// establish that the bytes belong to the expected live object, are valid for `T`, and are not
/// written concurrently. A page query does not prove those conditions or hold the mapping open.
pub unsafe fn rd<T: Copy>(addr: usize) -> Option<T> {
    let end = addr.checked_add(std::mem::size_of::<T>())?;
    let last = end - 1;
    if addr < 0x1_0000 || !readable(addr) {
        return None;
    }
    // The first byte is mapped above; the last byte needs its own check only when the read spans two pages.
    let spans_two_pages = last / PAGE_SIZE != addr / PAGE_SIZE;
    if spans_two_pages && !readable(last) {
        return None;
    }
    Some((addr as *const T).read_unaligned())
}

/// Check every page of a bounded native span.
///
/// # Safety
/// Readability alone is not lifetime or identity. The caller must establish native ownership,
/// keep the span alive on its owning thread, and bound `bytes` before calling. Do not cache the
/// result across native mutations, callbacks or threads.
pub unsafe fn readable_span(address: usize, bytes: usize) -> bool {
    if address < 0x1_0000 {
        return false;
    }
    let Some(end) = address.checked_add(bytes) else {
        return false;
    };
    let mut cursor = address;
    while cursor < end {
        if !readable(cursor) {
            return false;
        }
        let Some(next_page) = (cursor / PAGE_SIZE + 1).checked_mul(PAGE_SIZE) else {
            return false;
        };
        cursor = next_page;
    }
    true
}

/// Read a bounded MSVC string as bytes, preserving the game's language encoding.
///
/// # Safety
/// The caller establishes the native object's identity/lifetime on the game thread.
/// Page checks detect malformed storage; they do not establish ownership.
pub unsafe fn native_string(address: usize) -> Option<CString> {
    let length = rd::<usize>(address + 0x10)?;
    let capacity = rd::<usize>(address + 0x18)?;
    if length > 256 || length > capacity {
        return None;
    }
    let data = if capacity > 15 {
        rd::<usize>(address)?
    } else {
        address
    };
    if !readable_span(data, length + 1) || read::<u8>(data + length) != 0 {
        return None;
    }
    let bytes = std::slice::from_raw_parts(data as *const u8, length);
    CString::new(bytes).ok()
}
/// Read a native vector header and bound the borrowed element span. Game thread only.
pub unsafe fn vector_span(header: usize, stride: usize, limit: usize) -> Option<(usize, usize)> {
    let begin = rd::<usize>(header)?;
    let end = rd::<usize>(header + size_of::<usize>())?;
    let capacity = rd::<usize>(header + 2 * size_of::<usize>())?;
    let bytes = end.checked_sub(begin)?;
    if begin == 0
        || !begin.is_multiple_of(size_of::<usize>())
        || end > capacity
        || !bytes.is_multiple_of(stride)
        || bytes / stride > limit
        || !readable_span(begin, bytes)
    {
        return None;
    }
    Some((begin, bytes / stride))
}

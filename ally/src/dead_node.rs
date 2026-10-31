//! Read-only text diagnostic for the native dead-node/master assertion.
//! Formerly bundled with battle capture; now controlled by logging.dead_node.

use patchy::{PatchSession, ReturnType, Trampoline};
use std::{
    fmt::Write as _,
    io::Write as _,
    sync::atomic::{AtomicU32, Ordering},
};

use crate::{diag::rd, plausible, read, roster, CLOCK};

// Steam 1.163 collector 0x1401be250: report just before its fatal master-link test.
// Preserve the original instructions; do not suppress the native assertion.
const DEAD_NODE_SITE: usize = 0x1_401b_e274;
const DEAD_NODE_BYTES: [u8; 8] = [0x48, 0x83, 0xbf, 0x88, 0x00, 0x00, 0x00, 0x00];
const DEAD_JOINT_SITE: usize = 0x1_401b_e2f8;
const DEAD_JOINT_BYTES: [u8; 5] = [0x48, 0x83, 0x7f, 0x58, 0x00];
const OFF_PARTS: (usize, usize) = (0x1330, 0x1338);
const OFF_NAME: usize = 0xa90;
static DEAD_REPORTS: AtomicU32 = AtomicU32::new(0);

/// Adds byte-validated detours to the caller's patch session.
///
/// # Safety
/// Requires the supported Steam 1.163 executable at its verified image base,
/// with patch installation synchronized as for the other native hooks.
pub unsafe fn install_dead_node_hooks(session: &mut PatchSession) -> Result<(), String> {
    for (site, bytes, hook) in [
        (
            DEAD_NODE_SITE,
            &DEAD_NODE_BYTES[..],
            on_dead_node as *const (),
        ),
        (
            DEAD_JOINT_SITE,
            &DEAD_JOINT_BYTES[..],
            on_dead_joint as *const (),
        ),
    ] {
        let found = std::slice::from_raw_parts(site as *const u8, bytes.len());
        if found != bytes {
            return Err(format!(
                "dead-node site {site:#x} has unexpected bytes {found:02x?}"
            ));
        }
        let mut trampoline = Trampoline::new();
        trampoline.preserved_call(hook, &[0x48, 0x8b, 0xcf], &[], ReturnType::None); // mov rcx, rdi
        trampoline.bytes(bytes);
        trampoline.relative_jump(site + bytes.len());
        session
            .detour_trampoline(site, bytes.len(), trampoline)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

unsafe extern "C" fn on_dead_node(node: usize) {
    let _ = std::panic::catch_unwind(|| {
        if plausible(node) && rd::<usize>(node + 0x88).unwrap_or(0) != 0 {
            report_dead("node", node, 0x88, 0x90);
        }
    });
}

unsafe extern "C" fn on_dead_joint(node: usize) {
    let _ = std::panic::catch_unwind(|| {
        if plausible(node) && rd::<usize>(node + 0x58).unwrap_or(0) != 0 {
            report_dead("joint", node, 0x58, 0x60);
        }
    });
}

// Borrow only bounded scalar bytes from game-owned MSVC strings, never their ownership.
unsafe fn read_string(addr: usize, max: usize) -> Option<String> {
    let len = rd::<usize>(addr + 0x10)?;
    let cap = rd::<usize>(addr + 0x18)?;
    if len == 0 || len > max || cap < len || cap > 0x1000 {
        return None;
    }
    let data = if cap > 0xf { rd::<usize>(addr)? } else { addr };
    if !plausible(data) && cap > 0xf {
        return None;
    }
    let mut text = String::with_capacity(len);
    for i in 0..len {
        let byte = rd::<u8>(data + i)?;
        if !(0x20..0x7f).contains(&byte) {
            return None;
        }
        text.push(if byte == b',' || byte == b'"' {
            ';'
        } else {
            byte as char
        });
    }
    Some(text)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Class name from MSVC x64 RTTI: vtable[-1] is the complete object locator.
unsafe fn class_name(object: usize) -> String {
    let Some(vtable) = rd::<usize>(object).filter(|value| plausible(*value)) else {
        return "?".into();
    };
    let Some(locator) = rd::<usize>(vtable - 8).filter(|value| plausible(*value)) else {
        return format!("vtable {vtable:#x} (no rtti)");
    };
    let Some(type_rva) = rd::<u32>(locator + 0xc) else {
        return format!("vtable {vtable:#x}");
    };
    let mut name = String::new();
    for i in 0..80 {
        match rd::<u8>(0x1_4000_0000 + type_rva as usize + 0x10 + i) {
            Some(byte) if (0x20..0x7f).contains(&byte) => name.push(byte as char),
            _ => break,
        }
    }
    format!(
        "{name} (vtable {vtable:#x}, rva {:#x})",
        vtable.wrapping_sub(0x1_4000_0000)
    )
}

unsafe fn report_dead(kind: &str, node: usize, master_off: usize, master_id_off: usize) {
    if DEAD_REPORTS.fetch_add(1, Ordering::Relaxed) >= 20 {
        return;
    }
    let master = rd::<usize>(node + master_off).unwrap_or(0);
    let mut text = String::new();
    let _ = writeln!(
        text,
        "DEADNODE {kind} {node:#x} t={:.2} v{}",
        read::<f64>(CLOCK),
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(text, "  class: {}", class_name(node));
    let _ = writeln!(
        text,
        "  state(+0x48)={:?} m_id(+0x78)={:?} master ptr={master:#x} master id={:?}",
        rd::<i32>(node + 0x48),
        rd::<i64>(node + 0x78),
        rd::<i64>(node + master_id_off)
    );
    let _ = writeln!(
        text,
        "  master class: {}",
        if plausible(master) {
            class_name(master)
        } else {
            "?".into()
        }
    );
    let _ = writeln!(text, "  name(+0xc8)={:?}", read_string(node + 0xc8, 40));
    for (i, ship) in roster().into_iter().enumerate() {
        let (begin, end): (usize, usize) = (read(ship + OFF_PARTS.0), read(ship + OFF_PARTS.1));
        if begin == 0 || end < begin || (end - begin) / 0x18 > 4000 {
            continue;
        }
        for (part, address) in (begin..end).step_by(0x18).enumerate() {
            if rd::<usize>(address + 8) == Some(node) {
                let _ = writeln!(
                    text,
                    "  IS part {part} of roster ship {i} ({ship:#x}, name {:?})",
                    read_string(ship + OFF_NAME, 40)
                );
            }
        }
    }
    if let Some(raw) = rd::<[u8; 256]>(node) {
        for (i, chunk) in raw.chunks(32).enumerate() {
            let _ = writeln!(text, "  +{:#05x}: {}", i * 32, hex(chunk));
        }
    }
    log::error!("{}", text.trim_end());
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("Modloader/logs/hf_ally_deadnode.log")
    {
        let _ = file.write_all(text.as_bytes());
        let _ = file.sync_all();
    }
}

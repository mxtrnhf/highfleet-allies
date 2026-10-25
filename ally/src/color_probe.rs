//! `log_colors` (diagnostic): logs every distinct colour the game hands to its sprite-colour setter `0x140172ae0`
//! (RCX sprite, EDX ARGB, R8D corner), its colour-vector setter `0x140016fe0` (RCX out float4*, EDX ARGB) and its font style
//! call `0x1401a8710` (name, scale, blend, colour in R9D), with the return address of the caller. Used to read off the
//! game's friendly blue and the yellow of the combat hover callout (notes `findings/battle-hud-callouts.md` section 4).

use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

use patchy::{Condition, PatchSession, ReturnType, Trampoline};

const SPRITE_COLOR: usize = 0x1_4017_2ae0;
const SPRITE_BYTES: [u8; 9] = [0x41, 0x83, 0xf8, 0xff, 0x74, 0x0d, 0x49, 0x63, 0xc0];
const VECTOR_COLOR: usize = 0x1_4001_6fe0;
const VECTOR_BYTES: [u8; 8] = [0xf3, 0x0f, 0x10, 0x0d, 0xa0, 0xd8, 0x33, 0x00];
const VECTOR_CONST: u64 = 0x1_4035_4888;
const STYLE_FN: usize = 0x1_401a_8710;
const STYLE_BYTES: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10];

/// The return address of the call being logged, stored by the trampoline before the handler runs.
/// handle (the loaded object pointer) -> resource name, read from the HGE resource manager's lists (see `scan_names`).
static NAMES: Mutex<Option<std::collections::HashMap<usize, String>>> = Mutex::new(None);

unsafe fn ascii_name(p: usize) -> Option<String> {
    if !crate::plausible(p) {
        return None;
    }
    let bytes = std::slice::from_raw_parts(p as *const u8, 64);
    let end = bytes.iter().position(|b| *b == 0)?;
    if end < 3 || !bytes[..end].iter().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

/// Reads the resource manager `*0x143a119e8` (standard HGE: `res[RESTYPES]` heads of `ResDesc` lists; a ResDesc is
/// {vtable, char name[128], int resgroup, handle, next}) and builds the pointer -> name map. The layout is checked, not
/// assumed: heads are any qword in the manager that points to a node with a printable name at +8, and the chain is walked
/// through `next` at +0x98. Logs what it found so the offsets can be confirmed.
pub unsafe fn scan_names() {
    let manager: usize = (0x1_43a1_19e8usize as *const usize).read_unaligned();
    if !crate::plausible(manager) {
        return;
    }
    let mut map = std::collections::HashMap::new();
    let mut heads = 0;
    for off in (0..0x200).step_by(8) {
        let head: usize = ((manager + off) as *const usize).read_unaligned();
        if ascii_name(head + 8).is_none() {
            continue;
        }
        heads += 1;
        let mut node = head;
        for _ in 0..4000 {
            let Some(name) = ascii_name(node + 8) else { break };
            // the loaded object may sit at any qword after the name (the layout is not known): index all of them
            for off in [0x88usize, 0x90, 0xa0, 0xa8, 0xb0] {
                let handle: usize = ((node + off) as *const usize).read_unaligned();
                if handle > 0x1_0000 {
                    map.entry(handle).or_insert_with(|| name.clone());
                }
            }
            node = ((node + 0x98) as *const usize).read_unaligned();
            if !crate::plausible(node) {
                break;
            }
        }
    }
    if NAMES_LOGS.fetch_add(1, Ordering::Relaxed) < 3 {
        log::info!("COLOR names: manager {manager:#x}, {heads} lists, {} named handles", map.len());
    }
    *NAMES.lock().unwrap() = Some(map);
}

fn name_of(handle: usize) -> String {
    NAMES.try_lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&handle).cloned())).map(|n| format!(" name {n:?}")).unwrap_or_default()
}

static NAMES_LOGS: AtomicUsize = AtomicUsize::new(0);
static STASH: AtomicUsize = AtomicUsize::new(0);
static SEEN: Mutex<Option<HashSet<(u8, u32, usize, usize)>>> = Mutex::new(None);
const MAX_ENTRIES: usize = 1500;

fn note(kind: u8, argb: u32, sprite: usize, detail: &str) {
    let ret = STASH.load(Ordering::Relaxed);
    // font glyph recolouring (Font_SetColor, 256 glyph sprites per call) and the text renderer would fill the log: skip them
    if kind == 0 && matches!(ret, 0x1_4016_f155 | 0x1_4017_8e4b) {
        return;
    }
    let Ok(mut guard) = SEEN.try_lock() else { return };
    let set = guard.get_or_insert_with(HashSet::new);
    if set.len() >= MAX_ENTRIES || !set.insert((kind, argb & 0x00ff_ffff, ret, sprite)) {
        return;
    }
    let what = match kind {
        0 => "sprite",
        1 => "vector",
        _ => "style",
    };
    log::info!("COLOR {what} argb={argb:#010x} ret={ret:#x} (rva {:#x}) {detail}", ret.wrapping_sub(0x1_4000_0000));
}

unsafe extern "C" fn on_sprite(sprite: usize, argb: u32, corner: i32) {
    // which of the world-marker sprites (globals 0x143a2beb0..0x143a2bee8: sign_shell_*, torpedo, missile, drone, prize at
    // 0x143a2bee8) and the popup leader line (0x14800eb70) is being coloured
    let mut which = String::new();
    for (i, slot) in (0x1_43a2_beb0usize..=0x1_43a2_bee8).step_by(8).enumerate() {
        if rd_ptr(slot) == sprite && sprite != 0 {
            which = format!(" marker-slot {i} ({slot:#x})");
        }
    }
    if rd_ptr(0x1_4800_eb70) == sprite && sprite != 0 {
        which = " leader-line".to_string();
    }
    note(0, argb, sprite, &format!("sprite {sprite:#x} corner {corner}{which}{}", name_of(sprite)));
}

unsafe fn rd_ptr(a: usize) -> usize {
    (a as *const usize).read_unaligned()
}

unsafe extern "C" fn on_vector(_out: usize, argb: u32) {
    note(1, argb, 0, "");
}

unsafe extern "C" fn on_style(name: usize, _scale: f32, blend: i32, argb: u32) {
    let text = if crate::plausible(name) {
        let bytes = std::slice::from_raw_parts(name as *const u8, 40);
        String::from_utf8_lossy(&bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(40)]).into_owned()
    } else {
        String::new()
    };
    note(2, argb, 0, &format!("font {text:?} blend {blend}"));
}

/// Machine code that stores the return address of the hooked call (at `[rsp]` on entry) into `STASH` (clobbers RAX, R11:
/// both volatile and never arguments at a function entry).
fn stash_return(t: &mut Trampoline) {
    t.bytes(&[0x48, 0x8b, 0x04, 0x24]); // mov rax,[rsp]
    t.bytes(&[0x49, 0xbb]); // mov r11, imm64
    t.bytes(&(STASH.as_ptr() as u64).to_le_bytes());
    t.bytes(&[0x49, 0x89, 0x03]); // mov [r11], rax
}

fn call(t: &mut Trampoline, f: *const ()) {
    t.bytes(&[0x48, 0x83, 0xec, 0x08]); // sub rsp,8: entry RSP is 8 mod 16
    t.preserved_call(f, &[], &[], ReturnType::None);
    t.bytes(&[0x48, 0x83, 0xc4, 0x08]); // add rsp,8
}

pub unsafe fn install(session: &mut PatchSession) {
    let check = |addr: usize, want: &[u8]| std::slice::from_raw_parts(addr as *const u8, want.len()) == want;
    if check(SPRITE_COLOR, &SPRITE_BYTES) {
        let mut t = Trampoline::new();
        stash_return(&mut t);
        call(&mut t, on_sprite as *const ());
        let corner_all = t.new_label();
        t.bytes(&[0x41, 0x83, 0xf8, 0xff]); // cmp r8d,-1
        t.jump_if(Condition::Equal, corner_all);
        t.bytes(&[0x49, 0x63, 0xc0]); // movsxd rax,r8d
        t.relative_jump(SPRITE_COLOR + SPRITE_BYTES.len());
        let _ = t.bind(corner_all);
        t.absolute_jump(0x1_4017_2af3);
        if let Err(e) = session.detour_trampoline(SPRITE_COLOR, SPRITE_BYTES.len(), t) {
            log::warn!("hf_ally: colour probe (sprite) failed: {e}");
        }
    } else {
        log::warn!("hf_ally: colour probe: bytes at {SPRITE_COLOR:#x} differ");
    }
    if check(VECTOR_COLOR, &VECTOR_BYTES) {
        let mut t = Trampoline::new();
        stash_return(&mut t);
        call(&mut t, on_vector as *const ());
        t.bytes(&[0x48, 0xb8]); // mov rax, imm64: the RIP-relative load below, rewritten (rax is overwritten two instructions on)
        t.bytes(&VECTOR_CONST.to_le_bytes());
        t.bytes(&[0xf3, 0x0f, 0x10, 0x08]); // movss xmm1,[rax]
        t.relative_jump(VECTOR_COLOR + VECTOR_BYTES.len());
        if let Err(e) = session.detour_trampoline(VECTOR_COLOR, VECTOR_BYTES.len(), t) {
            log::warn!("hf_ally: colour probe (vector) failed: {e}");
        }
    } else {
        log::warn!("hf_ally: colour probe: bytes at {VECTOR_COLOR:#x} differ");
    }
    if check(STYLE_FN, &STYLE_BYTES) {
        let mut t = Trampoline::new();
        stash_return(&mut t);
        call(&mut t, on_style as *const ());
        t.bytes(&STYLE_BYTES);
        t.relative_jump(STYLE_FN + STYLE_BYTES.len());
        if let Err(e) = session.detour_trampoline(STYLE_FN, STYLE_BYTES.len(), t) {
            log::warn!("hf_ally: colour probe (style) failed: {e}");
        }
    } else {
        log::warn!("hf_ally: colour probe: bytes at {STYLE_FN:#x} differ");
    }
    log::info!("hf_ally: colour probe on (log_colors)");
}

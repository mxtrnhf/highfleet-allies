//! ClickFunc and SomethingInputTask can each have only one native detour.
//! The first enabled participant to commit owns both. Other DLLs register
//! their callbacks through a versioned, filename-independent C ABI.

use std::{
    mem::{size_of, size_of_val},
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

use windows_sys::Win32::{
    Foundation::HMODULE,
    System::{
        LibraryLoader::GetProcAddress, ProcessStatus::K32EnumProcessModules,
        Threading::GetCurrentProcess,
    },
};

use crate::patchy::{PatchSession, ReturnType, Trampoline};

const INPUT_TASK: usize = 0x1_4018_dbf0;
const INPUT_TASK_PROLOGUE: [u8; 11] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x10,
];
const CLICK_ENTRY: usize = 0x1_4012_f120;
const CLICK_STOLEN: [u8; 26] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac,
    0x24, 0xf8, 0xd3, 0xff, 0xff, 0xb8, 0x08, 0x2d, 0x00, 0x00,
];

pub const ACES: usize = 0;
pub const CCU: usize = 1;
pub const ALLY: usize = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Callbacks {
    pub tick: unsafe extern "C" fn(),
    pub click: unsafe extern "C" fn(usize, usize, usize, usize) -> bool,
    pub swap_mask: Option<extern "C" fn(u32) -> u32>,
}

#[repr(C)]
pub struct GuiHost {
    available: extern "C" fn(usize) -> bool,
    register: unsafe extern "C" fn(usize, *const Callbacks) -> bool,
    swap_mask: unsafe extern "C" fn(u32, *mut u32) -> bool,
}

static CALLBACKS: [OnceLock<Callbacks>; 3] = [const { OnceLock::new() }; 3];
static OWNER: AtomicBool = AtomicBool::new(false);
static HOST: OnceLock<&'static GuiHost> = OnceLock::new();
static LOCAL_HOST: GuiHost = GuiHost {
    available,
    register,
    swap_mask,
};

#[doc(hidden)]
pub fn local_host() -> *const GuiHost {
    if OWNER.load(Ordering::Acquire) {
        &LOCAL_HOST
    } else {
        std::ptr::null()
    }
}

/// Export the in-process GUI registration entry point from a participating DLL.
#[macro_export]
macro_rules! export_gui_host {
    () => {
        #[no_mangle]
        extern "C" fn hf_allies_gui_host_v1() -> *const $crate::gui_hooks::GuiHost {
            $crate::gui_hooks::local_host()
        }
    };
}

extern "C" fn available(role: usize) -> bool {
    CALLBACKS.get(role).is_some_and(|slot| slot.get().is_none())
}

unsafe extern "C" fn register(role: usize, callbacks: *const Callbacks) -> bool {
    // SAFETY: A v1 participant passes its initialized C table for this call.
    CALLBACKS
        .get(role)
        .is_some_and(|slot| !callbacks.is_null() && slot.set(callbacks.read()).is_ok())
}

unsafe extern "C" fn swap_mask(mask: u32, previous: *mut u32) -> bool {
    let Some(swap) = CALLBACKS[ALLY]
        .get()
        .and_then(|callbacks| callbacks.swap_mask)
    else {
        return false;
    };
    // SAFETY: A v1 participant passes a writable u32 on its own stack.
    previous.write(swap(mask));
    true
}

pub fn replace_ally_mask(mask: u32) -> Option<u32> {
    let mut previous = 0;
    let host = HOST.get()?;
    // SAFETY: The output is on our stack and the loader retains all registered DLLs.
    unsafe { (host.swap_mask)(mask, &mut previous) }.then_some(previous)
}

unsafe fn find_owner() -> Result<Option<&'static GuiHost>, String> {
    let mut modules: Vec<HMODULE> = vec![std::ptr::null_mut(); 256];
    let mut needed = 0;
    loop {
        let bytes = size_of_val(modules.as_slice()) as u32;
        if K32EnumProcessModules(
            GetCurrentProcess(),
            modules.as_mut_ptr(),
            bytes,
            &mut needed,
        ) == 0
        {
            return Err(format!(
                "cannot enumerate GUI hook participants: {}",
                std::io::Error::last_os_error()
            ));
        }
        if needed <= bytes {
            modules.truncate(needed as usize / size_of::<HMODULE>());
            break;
        }
        let count = needed as usize / size_of::<HMODULE>();
        if count > 4096 {
            return Err("too many loaded modules to locate the GUI hook owner".into());
        }
        modules.resize(count, std::ptr::null_mut());
    }
    for module in modules {
        if let Some(address) = GetProcAddress(module, c"hf_allies_gui_host_v1".as_ptr().cast()) {
            // SAFETY: Only our protocol participants export this exact versioned name.
            // Modloader initializes them serially and keeps every DLL loaded.
            let get_host: extern "C" fn() -> *const GuiHost = std::mem::transmute(address);
            if let Some(host) = get_host().as_ref() {
                return Ok(Some(host));
            }
        }
    }
    Ok(None)
}

/// Stage the shared seams only if unowned, commit all of this DLL's patches,
/// then publish callbacks. No other DLL can see a partially installed owner.
///
/// # Safety
/// Call once from serialized Steam 1.163 initialization at the known image base.
/// The callbacks borrow live game state only during the native game-thread call.
pub unsafe fn commit(
    mut session: PatchSession,
    role: usize,
    callbacks: Callbacks,
) -> Result<(), String> {
    let owner = find_owner()?;
    let host = owner.unwrap_or(&LOCAL_HOST);
    if !(host.available)(role) {
        return Err(
            "this mod already registered its GUI callbacks; remove the duplicate DLL".into(),
        );
    }
    if owner.is_none() {
        install(&mut session)?;
    }
    session.install_permanently().map_err(|e| e.to_string())?;
    if !(host.register)(role, &callbacks) {
        return Err("GUI callback registration failed after patch commit".into());
    }
    let _ = HOST.set(host);
    if owner.is_none() {
        OWNER.store(true, Ordering::Release);
        log::info!("shared GUI: installed ClickFunc and SomethingInputTask");
    } else {
        log::info!("shared GUI: joined existing ClickFunc and SomethingInputTask owner");
    }
    Ok(())
}

unsafe extern "C" fn on_input_frame() {
    for callbacks in CALLBACKS.iter().filter_map(OnceLock::get) {
        (callbacks.tick)();
    }
}

unsafe extern "C" fn on_click(ctx: usize, gui: usize, button: usize, id: usize) -> bool {
    gui != 0
        && CALLBACKS
            .iter()
            .filter_map(OnceLock::get)
            .any(|callbacks| (callbacks.click)(ctx, gui, button, id))
}

unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    // At ClickFunc entry: retain all four arguments, call the dispatcher, then
    // return for a consumed click or replay the native prologue and resume.
    let mut code = vec![
        0x51, 0x52, 0x41, 0x50, 0x41, 0x51, 0x48, 0x83, 0xec, 0x28, 0x48, 0xb8,
    ];
    code.extend((on_click as *const () as usize as u64).to_le_bytes());
    code.extend([
        0xff, 0xd0, 0x48, 0x83, 0xc4, 0x28, 0x84, 0xc0, 0x41, 0x59, 0x41, 0x58, 0x5a, 0x59,
    ]);
    code.extend([0x75, (CLICK_STOLEN.len() + 14) as u8]);
    code.extend(CLICK_STOLEN);
    code.extend([0xff, 0x25, 0, 0, 0, 0]);
    code.extend(((CLICK_ENTRY + CLICK_STOLEN.len()) as u64).to_le_bytes());
    code.push(0xc3);
    let mut click = Trampoline::new();
    click.bytes(&code);
    session
        .detour_trampoline(CLICK_ENTRY, CLICK_STOLEN.len(), click)
        .map_err(|e| e.to_string())?;

    // Function entry has RSP at 8 mod 16; align before the preserved call.
    let mut frame = Trampoline::new();
    frame.bytes(&[0x48, 0x83, 0xec, 0x08]);
    frame.preserved_call(on_input_frame as *const (), &[], &[], ReturnType::None);
    frame.bytes(&[0x48, 0x83, 0xc4, 0x08]);
    frame.bytes(&INPUT_TASK_PROLOGUE);
    frame.relative_jump(INPUT_TASK + INPUT_TASK_PROLOGUE.len());
    session
        .detour_trampoline(INPUT_TASK, INPUT_TASK_PROLOGUE.len(), frame)
        .map_err(|e| e.to_string())?;
    Ok(())
}

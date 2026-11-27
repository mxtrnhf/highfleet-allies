use std::{
    ffi::{c_char, CStr},
    io::{ErrorKind, Write},
    sync::atomic::{AtomicBool, Ordering},
};

use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;

static SUPPORTED: AtomicBool = AtomicBool::new(false);

/// Accept the Modloader game version for this DLL.
///
/// # Safety
/// `version` points to a NUL-terminated loader string for this call.
pub unsafe fn version(version: *const c_char) -> bool {
    let supported = !version.is_null() && CStr::from_ptr(version).to_bytes() == b"Steam 1.163";
    SUPPORTED.store(supported, Ordering::Relaxed);
    supported
}

pub fn check_game() -> Result<(), String> {
    if !SUPPORTED.load(Ordering::Relaxed) {
        return Err("requires HighFleet Steam 1.163".into());
    }
    // SAFETY: Null asks Windows for the current executable's handle.
    if unsafe { GetModuleHandleA(std::ptr::null()) } as usize != crate::native::IMAGE_BASE {
        return Err("unexpected executable image base".into());
    }
    Ok(())
}

/// Read the JSON config, creating a default file on the first launch.
pub fn config<T>(path: &str) -> Result<T, String>
where
    T: Default + serde::Serialize + serde::de::DeserializeOwned,
{
    let result = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let defaults = T::default();
            // Preserve a config created between the read and the create attempt.
            let bytes = serde_json::to_vec_pretty(&defaults).map_err(|e| e.to_string())?;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(mut file) => file
                    .write_all(&bytes)
                    .and_then(|()| file.write_all(b"\n"))
                    .map(|()| defaults)
                    .map_err(|e| e.to_string()),
                Err(error) => Err(error.to_string()),
            }
        }
        Err(error) => Err(error.to_string()),
    };
    result.map_err(|error| format!("{path}: {error}"))
}

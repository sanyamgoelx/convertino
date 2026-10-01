//! The user's system accent colour, so the wheel matches the OS.

/// "#RRGGBB", or None to let the wheel use its default.
#[cfg(windows)]
pub fn system_accent() -> Option<String> {
    use ::windows::core::w;
    use ::windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // DWM stores the accent as 0xAABBGGRR.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\DWM"),
            w!("AccentColor"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let r = value & 0xFF;
    let g = (value >> 8) & 0xFF;
    let b = (value >> 16) & 0xFF;
    Some(format!("#{r:02X}{g:02X}{b:02X}"))
}

/// macOS: the wheel's CSS uses the system accent keyword instead.
#[cfg(not(windows))]
pub fn system_accent() -> Option<String> {
    None
}

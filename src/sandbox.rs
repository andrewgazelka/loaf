//! macOS Seatbelt sandbox via direct FFI to libsystem_sandbox.dylib
//!
//! This provides process-level sandboxing using Apple's Seatbelt framework.
//! The sandbox_init() function is deprecated but still functional and is used
//! by major applications like Chromium and Firefox.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;

// FFI bindings to libsystem_sandbox.dylib (part of System framework)
unsafe extern "C" {
    /// Apply sandbox profile to current process.
    /// Returns 0 on success, -1 on failure.
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> i32;

    /// Free error buffer returned by sandbox_init.
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// Profile type flag - 0 means raw SBPL string (Sandbox Profile Language)
/// Other flags: SANDBOX_NAMED (0x0001), SANDBOX_NAMED_BUILTIN (0x0002)
const SANDBOX_NAMED_EXTERNAL: u64 = 0x0000;

/// Apply sandbox to current process. Call this AFTER fork, BEFORE exec.
///
/// # Safety
/// Must be called in child process before exec. Not thread-safe.
/// The sandbox cannot be removed once applied.
pub unsafe fn apply_sandbox(profile: &str) -> Result<(), String> {
    let profile_cstr = CString::new(profile).map_err(|e| format!("invalid profile string: {e}"))?;

    let mut error: *mut c_char = std::ptr::null_mut();

    let result = unsafe { sandbox_init(profile_cstr.as_ptr(), SANDBOX_NAMED_EXTERNAL, &mut error) };

    if result != 0 {
        let err_msg = if !error.is_null() {
            let msg = unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned();
            unsafe { sandbox_free_error(error) };
            msg
        } else {
            "unknown sandbox error".to_string()
        };
        return Err(err_msg);
    }

    Ok(())
}

/// Generate SBPL profile that protects the base path (project directory).
///
/// Policy:
/// - DENY writes to base_path (the real project dir) - must go through NFS overlay
/// - ALLOW everything else (Claude needs ~/.claude, /tmp, network, etc.)
///
/// This is a safety net to prevent bypassing the overlay via absolute paths.
pub fn generate_profile(base_path: &Path) -> String {
    let base = base_path.display();

    format!(
        r#"(version 1)
(allow default)

;; DENY writes to the project directory
;; These must go through the NFS overlay mount point
(deny file-write* (subpath "{base}"))
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_profile() {
        let profile = generate_profile(std::path::Path::new("/Users/test/project"));
        assert!(profile.contains("(version 1)"));
        assert!(profile.contains("(allow default)"));
        assert!(profile.contains("(deny file-write*"));
        assert!(profile.contains("/Users/test/project"));
    }
}

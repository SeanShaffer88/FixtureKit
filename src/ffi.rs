//! C ABI surface — everything declared in include/fixturekit.h.
//! Conventions (house standard, from WinRecovery):
//! - `*mut c_char` results are caller-owned; free with `fx_free_string`.
//! - Failures return null / an ERR_* code + a thread-local last-error message.
//! - Nothing unwinds across the FFI boundary (catch_unwind everywhere;
//!   release profile also sets panic = "abort").

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::catch_unwind;

use crate::generate;
use crate::{ERR_INTERNAL, ERR_INVALID_ARG, FX_OK};

thread_local! {
    static LAST_ERROR: RefCell<(i32, CString)> =
        RefCell::new((0, CString::new("").unwrap()));
}

pub fn set_error(code: i32, msg: &str) {
    let clean: String = msg.chars().filter(|&c| c != '\0').collect();
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = (
            code,
            CString::new(clean).unwrap_or_else(|_| CString::new("error").unwrap()),
        );
    });
}

fn clear_error() {
    set_error(FX_OK, "");
}

#[no_mangle]
pub extern "C" fn fx_version() -> *const c_char {
    static VERSION_C: &[u8] = b"0.1.0\0";
    VERSION_C.as_ptr() as *const c_char
}

#[no_mangle]
pub extern "C" fn fx_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().1.as_ptr())
}

#[no_mangle]
pub extern "C" fn fx_last_error_code() -> c_int {
    LAST_ERROR.with(|e| e.borrow().0) as c_int
}

/// # Safety (caller contract): `p` is null or a string this library
/// returned and has not been freed yet. Documented in the header.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn fx_free_string(p: *mut c_char) {
    if !p.is_null() {
        unsafe {
            drop(CString::from_raw(p));
        }
    }
}

/// Generate the fixture `spec_json` describes. On success returns
/// a caller-owned result string (`{"path","kind","size","sha256"}`)
/// to free with `fx_free_string`; on failure returns null and
/// sets the thread-local last error.
///
/// # Safety (caller contract): `spec_json` is null or a valid
/// NUL-terminated UTF-8 string. Documented in the header.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn fx_generate(spec_json: *const c_char) -> *mut c_char {
    let outcome = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if spec_json.is_null() {
            set_error(ERR_INVALID_ARG, "spec_json must not be null");
            return std::ptr::null_mut();
        }
        let spec = match unsafe { CStr::from_ptr(spec_json) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                set_error(ERR_INVALID_ARG, "spec_json must be valid UTF-8");
                return std::ptr::null_mut();
            }
        };
        match generate::generate(spec) {
            Ok(result) => match CString::new(result) {
                Ok(s) => {
                    clear_error();
                    s.into_raw()
                }
                Err(_) => {
                    set_error(ERR_INTERNAL, "result string contained a NUL byte");
                    std::ptr::null_mut()
                }
            },
            Err((code, msg)) => {
                set_error(code, &msg);
                std::ptr::null_mut()
            }
        }
    }));
    outcome.unwrap_or_else(|_| {
        set_error(ERR_INTERNAL, "internal panic caught at the FFI boundary");
        std::ptr::null_mut()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;
    use std::ffi::CString;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fixturekit-ffi-{}-{}", std::process::id(), tag));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn call(spec: &str) -> Option<String> {
        let cs = CString::new(spec).unwrap();
        let p = fx_generate(cs.as_ptr());
        if p.is_null() {
            None
        } else {
            let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
            fx_free_string(p);
            Some(s)
        }
    }

    #[test]
    fn generate_end_to_end_and_error_paths() {
        let dir = tmpdir("e2e");
        let path = dir.join("tone.wav");
        let spec = format!(
            "{{\"kind\":\"wav\",\"path\":\"{}\",\"freq_hz\":440,\"duration_ms\":10,\"sample_rate\":8000}}",
            crate::json_escape(&path.to_string_lossy())
        );
        let result = call(&spec).expect("valid spec must succeed");
        let v = json::parse(&result).unwrap();
        assert_eq!(v.get("kind").unwrap().as_str(), Some("wav"));
        assert_eq!(v.get("size").unwrap().as_i64(), Some(44 + 160));
        assert!(path.exists());
        assert_eq!(fx_last_error_code(), FX_OK as c_int);

        // Null and malformed specs fail with the error channel set.
        assert!(fx_generate(std::ptr::null()).is_null());
        assert_eq!(fx_last_error_code(), ERR_INVALID_ARG as c_int);
        assert!(call("{not json").is_none());
        assert_eq!(fx_last_error_code(), ERR_INVALID_ARG as c_int);
        let msg = unsafe { CStr::from_ptr(fx_last_error()) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(msg.contains("not valid JSON"), "{msg}");
    }

    #[test]
    fn version_is_the_crate_version() {
        let v = unsafe { CStr::from_ptr(fx_version()) }.to_str().unwrap();
        assert_eq!(v, crate::VERSION);
    }
}

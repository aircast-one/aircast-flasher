use security_framework_sys::authorization::*;
use std::ffi::CString;
use std::ptr;
use std::sync::Mutex;

static EXEC_LOCK: Mutex<()> = Mutex::new(());

pub struct Authorization {
    auth_ref: AuthorizationRef,
}

impl Authorization {
    pub fn new() -> Result<Self, String> {
        let mut auth_ref: AuthorizationRef = ptr::null_mut();
        let status = unsafe {
            AuthorizationCreate(
                ptr::null(),
                ptr::null(),
                kAuthorizationFlagDefaults,
                &mut auth_ref,
            )
        };
        if status != 0 {
            return Err(format!("AuthorizationCreate failed with status {status}"));
        }
        Ok(Self { auth_ref })
    }

    pub fn execute(&self, tool: &str, args: &[&str]) -> Result<String, String> {
        let tool_c = CString::new(tool).map_err(|e| format!("Invalid tool path: {e}"))?;
        let args_c: Vec<CString> = args
            .iter()
            .map(|a| CString::new(*a).map_err(|e| format!("Invalid argument: {e}")))
            .collect::<Result<Vec<_>, _>>()?;
        let mut args_ptrs: Vec<*mut libc::c_char> = args_c
            .iter()
            .map(|a| a.as_ptr() as *mut libc::c_char)
            .collect();
        args_ptrs.push(ptr::null_mut());

        let _guard = EXEC_LOCK
            .lock()
            .map_err(|e| format!("EXEC_LOCK poisoned: {e}"))?;

        let mut pipe: *mut libc::FILE = ptr::null_mut();
        #[allow(deprecated)]
        let status = unsafe {
            AuthorizationExecuteWithPrivileges(
                self.auth_ref,
                tool_c.as_ptr(),
                kAuthorizationFlagDefaults,
                args_ptrs.as_mut_ptr(),
                &mut pipe,
            )
        };
        if status != 0 {
            if status == -60006 {
                return Err("Authentication cancelled".to_string());
            }
            return Err(format!(
                "AuthorizationExecuteWithPrivileges failed with status {status}"
            ));
        }

        let mut output = String::new();
        if !pipe.is_null() {
            let mut buf = [0u8; 4096];
            loop {
                let n = unsafe {
                    libc::fread(buf.as_mut_ptr() as *mut libc::c_void, 1, buf.len(), pipe)
                };
                if n == 0 {
                    break;
                }
                output.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            unsafe { libc::fclose(pipe) };
        }

        let exit_code = reap_child()?;
        if exit_code != 0 {
            return Err(format!(
                "{tool} failed with exit code {exit_code}: {}",
                output.trim()
            ));
        }
        Ok(output)
    }
}

fn reap_child() -> Result<libc::c_int, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(900);
    loop {
        let mut status: libc::c_int = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid > 0 {
            return Ok(if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                -1
            });
        }
        if pid < 0 {
            if errno() == libc::ECHILD {
                return Ok(0);
            }
            return Err(format!("waitpid() failed (errno: {})", errno()));
        }
        if std::time::Instant::now() >= deadline {
            return Ok(0);
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
}

fn errno() -> i32 {
    unsafe { *libc::__error() }
}

impl Drop for Authorization {
    fn drop(&mut self) {
        unsafe {
            AuthorizationFree(self.auth_ref, kAuthorizationFlagDefaults);
        }
    }
}

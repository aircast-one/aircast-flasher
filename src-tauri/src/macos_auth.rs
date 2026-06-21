//! macOS Authorization Services wrapper for privileged command execution.
//!
//! Uses Apple's Authorization Services C API to show the standard system
//! authentication dialog (lock icon, Touch ID) and run commands as root. The app
//! never handles the user's password. Adapted from the proven aircast-web flasher.
//!
//! # TODO: migrate off `AuthorizationExecuteWithPrivileges`
//!
//! Apple deprecated this API in macOS 10.7. The modern replacement is a
//! separately code-signed privileged helper installed via `SMAppService`
//! (launchd daemon) or an XPC service, which requires a real Developer ID
//! certificate. Until this app is signed/notarized, this wrapper is the
//! pragmatic path, and it serializes spawn+wait pairs via `EXEC_LOCK` to avoid
//! the inherent `libc::wait()` race.

use security_framework_sys::authorization::*;
use std::ffi::CString;
use std::ptr;
use std::sync::Mutex;

/// Process-wide lock serializing AuthorizationExecuteWithPrivileges + wait()
/// pairs. `libc::wait(NULL)` reaps ANY child, not just the one this call
/// spawned; without this, two concurrent privileged executions would each reap
/// the other's child and return the wrong exit status.
static EXEC_LOCK: Mutex<()> = Mutex::new(());

/// RAII wrapper around `AuthorizationRef`. Shows the system auth dialog once and
/// reuses the authorization for multiple commands (e.g. unmount + write).
pub struct Authorization {
    auth_ref: AuthorizationRef,
}

impl Authorization {
    /// Create an authorization reference. Does not prompt yet — the dialog
    /// appears on the first `execute` call.
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

    /// Run `tool` (absolute path) with `args` as root. Shows the auth dialog on
    /// first call; later calls on the same instance reuse the grant.
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

        // Drain stdout to EOF; this also ensures the child has finished writing
        // before we wait().
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

/// Wait for a child spawned via AuthorizationExecuteWithPrivileges. Must be
/// called with EXEC_LOCK held. Returns the exit code, or -1 if signaled.
///
/// `AuthorizationExecuteWithPrivileges` doesn't expose the child PID, so we
/// can't `waitpid()` a specific process. A blocking `libc::wait()` deadlocks
/// when another reaper — notably tokio's SIGCHLD handler — has already
/// collected our child: `wait()` then blocks forever with nothing to reap.
/// Instead we poll with `WNOHANG` and treat `ECHILD` ("no children left",
/// i.e. it was reaped elsewhere) as successful completion. The caller has
/// already drained the child's stdout to EOF / closed its stdin, so by the
/// time we get here the child is finished — we only need its status.
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
            // ECHILD: our child was already reaped (e.g. by tokio) — done.
            if errno() == libc::ECHILD {
                return Ok(0);
            }
            return Err(format!("waitpid() failed (errno: {})", errno()));
        }
        // pid == 0: a child still exists but hasn't exited yet — wait briefly.
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

//! Single-writer PID lock (Invariant #5).
//!
//! On `Writer` open we create a `.agentwatch.lock` file in the data dir with
//! the current process PID. A second `agentwatch` invocation that wants write
//! access must either:
//!   1. find the lock file empty / stale (owning process gone) and claim it, or
//!   2. fail with `StoreError::AlreadyLocked` and tell the user.
//!
//! Read-only consumers (webapp, `report`, `summary`) do NOT acquire the lock;
//! they can run alongside the writer.

use std::fs;
use std::path::Path;

use crate::StoreError;

pub struct PidLock {
    path: std::path::PathBuf,
}

impl PidLock {
    pub fn acquire(dir: &Path) -> Result<Self, StoreError> {
        let path = dir.join(".agentwatch.lock");
        if let Ok(contents) = fs::read_to_string(&path) {
            if let Ok(pid) = contents.trim().parse::<u32>() {
                if pid_alive(pid) {
                    return Err(StoreError::AlreadyLocked { pid });
                }
            }
        }
        fs::write(&path, std::process::id().to_string())?;
        Ok(Self { path })
    }
}

impl Drop for PidLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // `kill -0` is the standard liveness check on unix: send no signal, just
    // probe whether the kernel knows the pid. Returns true if the kernel
    // recognizes it (alive, may not be ours), false on ESRCH (dead).
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    let err = std::io::Error::last_os_error().raw_os_error();
    err != Some(libc::ESRCH)
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    // Day 5 work: Windows OpenProcess check. Scaffold: assume alive to be safe.
    true
}

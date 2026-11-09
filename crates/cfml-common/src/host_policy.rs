//! Process-wide host-access policy: the sandbox bits that code without a VM
//! handle (stdlib builtins, Java shims, shared helpers) must still honour.
//!
//! The VM keeps its own `sandbox` field and refuses host-touching builtins by
//! name before dispatch (see `cfml-vm/src/host_policy.rs`). That gate is the
//! first line. This module is the second: the leaf operations themselves
//! consult it, so a code path that reaches a writer without passing the VM's
//! name gate (a shim backed by a builtin, a helper called from another
//! builtin) still cannot touch the host.
//!
//! Two independent switches:
//!
//! * **sandbox** — host filesystem writes and process execution are refused.
//!   Set by `--sandbox` and by `security.sandbox` in `.cfconfig.json`.
//! * **host reads confined** — whole-file reads through
//!   [`crate::vfs::read_regular_file`] are refused. Only a self-contained
//!   (`--build`) binary sets this, because its application lives in the
//!   embedded archive; a server running an application from disk must read it.
//!
//! Both switches only ever turn ON: there is no way for application code to
//! relax them once the process has started.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::vm::{CfmlError, CfmlErrorType};

static SANDBOX: AtomicBool = AtomicBool::new(false);
static HOST_READS_CONFINED: AtomicBool = AtomicBool::new(false);

/// Refuse host writes and process execution for the rest of the process.
pub fn enable_sandbox() {
    SANDBOX.store(true, Ordering::Release);
}

/// Whether [`enable_sandbox`] has been called.
#[inline]
pub fn is_sandboxed() -> bool {
    SANDBOX.load(Ordering::Acquire)
}

/// Refuse whole-file reads of host paths for the rest of the process.
pub fn confine_host_reads() {
    HOST_READS_CONFINED.store(true, Ordering::Release);
}

/// Whether [`confine_host_reads`] has been called.
#[inline]
pub fn host_reads_confined() -> bool {
    HOST_READS_CONFINED.load(Ordering::Acquire)
}

/// The error every refused host write carries. One wording, so callers and
/// tests can match on it.
pub fn write_refused(what: &str) -> CfmlError {
    CfmlError::new(
        format!("{what}: filesystem writes are disabled in sandbox mode"),
        CfmlErrorType::Custom("security".to_string()),
    )
}

/// The error every refused process launch carries.
pub fn exec_refused(what: &str) -> CfmlError {
    CfmlError::new(
        format!("{what}: process execution is disabled in sandbox mode"),
        CfmlErrorType::Custom("security".to_string()),
    )
}

/// The error every refused host read carries.
pub fn read_refused(what: &str) -> CfmlError {
    CfmlError::new(
        format!("{what}: host filesystem reads are disabled in sandbox mode"),
        CfmlErrorType::Custom("security".to_string()),
    )
}

/// `Err` when the process is sandboxed. Call at the top of anything that
/// creates, modifies or removes a host path.
#[inline]
pub fn check_host_write(what: &str) -> Result<(), CfmlError> {
    if is_sandboxed() {
        Err(write_refused(what))
    } else {
        Ok(())
    }
}

/// `Err` when the process is sandboxed. Call before launching a process.
#[inline]
pub fn check_host_exec(what: &str) -> Result<(), CfmlError> {
    if is_sandboxed() {
        Err(exec_refused(what))
    } else {
        Ok(())
    }
}

/// `Err` when host reads are confined. Call before reading a host path by a
/// route that does not go through the VM's virtual filesystem.
#[inline]
pub fn check_host_read(what: &str) -> Result<(), CfmlError> {
    if host_reads_confined() {
        Err(read_refused(what))
    } else {
        Ok(())
    }
}

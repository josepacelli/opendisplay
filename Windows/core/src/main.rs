// windows-core runs unattended, elevated, at logon — no console window
// should ever flash for it, regardless of how it's launched (Scheduled
// Task, double-click, or the installer).
#![cfg_attr(windows, windows_subsystem = "windows")]

//! windows-core entry point: bootstrap + elevation self-check.
//!
//! Design Risk mitigation ("UIPI silent failure if `windows-core` ever loses
//! elevation" — see `design.md`'s Risks & Concerns): before any other
//! subsystem (transport, capture, input, IPC) starts, `windows-core` checks
//! its own process token integrity level. `SendInput` and other UI-affecting
//! calls fail *silently* under UIPI when the caller isn't elevated enough,
//! so degrading quietly is exactly the failure mode to avoid — refusing to
//! start and reporting a clear status is the whole point of this check.
//!
//! The real Windows API call (`OpenProcessToken` + `GetTokenInformation`)
//! is isolated behind the `TokenIntegrityCheck` trait so the bootstrap
//! decision itself — "given this integrity level, should the rest of
//! windows-core start?" — is plain, unit-testable Rust with no OS call
//! involved.

mod capture;
mod display;
mod display_spec;
mod cursor;
mod encode;
mod frame_sender;
mod input;
mod ipc_server;
mod log;
mod protocol_session;
mod runtime;
mod session_state;
mod transport;

/// The process token integrity levels relevant to elevation, per
/// `SECURITY_MANDATORY_*_RID` in the Windows security model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrityLevel {
    Low,
    Medium,
    High,
    System,
}

impl IntegrityLevel {
    /// `windows-core` is considered elevated enough only at High or above —
    /// the level a Scheduled Task with `RunLevel=HighestAvailable` grants
    /// (`[[memory:AD-001]]`, `design.md`'s `windows-installer` component).
    pub fn is_elevated(self) -> bool {
        matches!(self, IntegrityLevel::High | IntegrityLevel::System)
    }
}

/// Abstraction over "what is this process's token integrity level", so the
/// bootstrap decision below is testable without calling into the real
/// Windows API.
pub trait TokenIntegrityCheck {
    fn current_integrity_level(&self) -> IntegrityLevel;
}

/// The real check, backed by the Windows API. Only compiled on Windows
/// targets; unverified by an automated gate on this host (no Rust/WDK
/// toolchain — see the task's Gate line), so a failure of any API call is
/// treated conservatively as "not elevated" rather than assumed elevated.
#[cfg(windows)]
pub struct WindowsTokenIntegrityCheck;

#[cfg(windows)]
impl TokenIntegrityCheck for WindowsTokenIntegrityCheck {
    fn current_integrity_level(&self) -> IntegrityLevel {
        windows_impl::query_integrity_level().unwrap_or(IntegrityLevel::Low)
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::IntegrityLevel;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Reads the current process's token integrity level. See MS docs for
    /// `GetTokenInformation(TokenIntegrityLevel)`: the returned
    /// `TOKEN_MANDATORY_LABEL.Label.Sid`'s last sub-authority is the
    /// `SECURITY_MANDATORY_*_RID` value.
    pub(super) fn query_integrity_level() -> windows::core::Result<IntegrityLevel> {
        unsafe {
            let process = GetCurrentProcess();
            let mut token = Default::default();
            OpenProcessToken(process, TOKEN_QUERY, &mut token)?;

            let mut len = 0u32;
            // First call is expected to fail with ERROR_INSUFFICIENT_BUFFER;
            // it exists only to learn the required buffer size.
            let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut len);
            let mut buf = vec![0u8; len as usize];
            let result = GetTokenInformation(
                token,
                TokenIntegrityLevel,
                Some(buf.as_mut_ptr() as *mut _),
                len,
                &mut len,
            );
            let _ = CloseHandle(token);
            result?;

            let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
            let sid = label.Label.Sid;
            let rid_count = *GetSidSubAuthorityCount(sid);
            let rid = *GetSidSubAuthority(sid, (rid_count as u32).saturating_sub(1));

            // SECURITY_MANDATORY_{LOW,MEDIUM,HIGH,SYSTEM}_RID.
            Ok(if rid >= 0x4000 {
                IntegrityLevel::System
            } else if rid >= 0x3000 {
                IntegrityLevel::High
            } else if rid >= 0x2000 {
                IntegrityLevel::Medium
            } else {
                IntegrityLevel::Low
            })
        }
    }
}

/// The result of `windows-core`'s startup self-check, decided before any
/// other subsystem starts. `NotElevated` is the status value a later IPC
/// layer (`ipc::serve`, T23) reports to `windows-tray` instead of
/// continuing degraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapStatus {
    /// Elevated at High integrity or above; the rest of windows-core may
    /// start.
    Ready,
    /// Not elevated; capture/input MUST NOT start.
    NotElevated,
}

/// Runs the startup elevation self-check using `checker`. Called before any
/// other subsystem (transport, capture, input, IPC) starts.
pub fn bootstrap(checker: &dyn TokenIntegrityCheck) -> BootstrapStatus {
    if checker.current_integrity_level().is_elevated() {
        BootstrapStatus::Ready
    } else {
        BootstrapStatus::NotElevated
    }
}

fn main() {
    #[cfg(windows)]
    {
        let status = bootstrap(&WindowsTokenIntegrityCheck);
        if status == BootstrapStatus::NotElevated {
            eprintln!(
                "windows-core: not running at High integrity; refusing to start \
                 capture/input. Reporting NotElevated status."
            );
            return;
        }
        runtime::run();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An injectable fake so the bootstrap decision is testable without any
    /// OS call, per the task's "injectable token-check abstraction".
    struct FakeIntegrityCheck(IntegrityLevel);

    impl TokenIntegrityCheck for FakeIntegrityCheck {
        fn current_integrity_level(&self) -> IntegrityLevel {
            self.0
        }
    }

    #[test]
    fn high_integrity_yields_ready() {
        let checker = FakeIntegrityCheck(IntegrityLevel::High);
        assert_eq!(bootstrap(&checker), BootstrapStatus::Ready);
    }

    #[test]
    fn system_integrity_yields_ready() {
        let checker = FakeIntegrityCheck(IntegrityLevel::System);
        assert_eq!(bootstrap(&checker), BootstrapStatus::Ready);
    }

    #[test]
    fn medium_integrity_yields_not_elevated() {
        let checker = FakeIntegrityCheck(IntegrityLevel::Medium);
        assert_eq!(bootstrap(&checker), BootstrapStatus::NotElevated);
    }

    #[test]
    fn low_integrity_yields_not_elevated() {
        let checker = FakeIntegrityCheck(IntegrityLevel::Low);
        assert_eq!(bootstrap(&checker), BootstrapStatus::NotElevated);
    }
}

//! Scheduled Task registration for windows-core (T30): registers
//! windows-core as a Task Scheduler task, `RunLevel=HighestAvailable`,
//! trigger `AtLogOn`, per `[[memory:AD-001]]` and design.md's "Elevated
//! autostart for windows-core" Tech Decision — the supported way to run
//! elevated at logon without a UAC prompt per launch (option C from
//! design.md's Architecture Overview, over a true Windows Service, which
//! can't reach the interactive desktop from Session 0).
//!
//! Per the Test Coverage Matrix, Task Scheduler registration is OS-bound
//! (Tests: none) — manual verification (log off/on, confirm High
//! integrity) is the only gate. The Task Scheduler COM API calls are
//! isolated behind a trait, same pattern as `driver_install`'s
//! `DriverPackageInstaller`.

use std::path::Path;

/// The Scheduled Task name windows-installer registers windows-core under.
pub const TASK_NAME: &str = "OpenDisplay Core";

/// A specific registration failure, so a caller can report something more
/// useful than "registration failed".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduledTaskError {
    /// The Task Scheduler COM service could not be reached.
    ServiceUnavailable,
    /// The task folder rejected the registration (e.g. insufficient
    /// privilege, invalid executable path).
    RegistrationRejected { detail: String },
}

impl std::fmt::Display for ScheduledTaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduledTaskError::ServiceUnavailable => {
                write!(f, "could not reach the Windows Task Scheduler service")
            }
            ScheduledTaskError::RegistrationRejected { detail } => {
                write!(f, "Task Scheduler rejected the registration: {detail}")
            }
        }
    }
}

impl std::error::Error for ScheduledTaskError {}

/// Abstraction over "register a Scheduled Task that launches
/// `executable_path` at logon, `RunLevel=HighestAvailable`", isolating the
/// Task Scheduler COM calls so registration could be exercised without a
/// real COM service. No test exists for this task (Tests: none, OS-bound
/// per the Test Coverage Matrix), but the isolation matches the pattern
/// used throughout this feature.
pub trait TaskScheduler {
    fn register_logon_task(
        &self,
        task_name: &str,
        executable_path: &Path,
    ) -> Result<(), ScheduledTaskError>;
}

/// Registers `executable_path` (windows-core's binary) as the
/// `AtLogOn`/`HighestAvailable` task, per this task's Done when ("after
/// registration, windows-core starts elevated at the next logon with no
/// UAC prompt").
pub fn register(
    scheduler: &dyn TaskScheduler,
    executable_path: &Path,
) -> Result<(), ScheduledTaskError> {
    scheduler.register_logon_task(TASK_NAME, executable_path)
}

/// The real registration, backed by the Task Scheduler COM API
/// (`ITaskService`). Not exercised by an automated gate on this host (no
/// Rust toolchain, no COM service to call) — verified manually by logging
/// off/on and confirming windows-core is running at High integrity.
#[cfg(windows)]
pub struct ComTaskScheduler;

#[cfg(windows)]
impl TaskScheduler for ComTaskScheduler {
    fn register_logon_task(
        &self,
        task_name: &str,
        executable_path: &Path,
    ) -> Result<(), ScheduledTaskError> {
        windows_impl::register_logon_task(task_name, executable_path)
            .map_err(|e| ScheduledTaskError::RegistrationRejected {
                detail: e.message(),
            })
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use windows::core::{Interface, BSTR, VARIANT};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::TaskScheduler::{
        IExecAction, ILogonTrigger, ITaskService, TaskScheduler as TaskSchedulerComClass,
        TASK_ACTION_EXEC, TASK_CREATE_OR_UPDATE, TASK_LOGON_INTERACTIVE_TOKEN,
        TASK_RUNLEVEL_HIGHEST, TASK_TRIGGER_LOGON,
    };

    /// Connects to the Task Scheduler COM service, builds a task
    /// definition with a `RunLevel=HighestAvailable` principal, a single
    /// `AtLogOn` trigger, and one exec action launching `executable_path`,
    /// then registers it under `task_name` in the root task folder.
    pub(super) fn register_logon_task(
        task_name: &str,
        executable_path: &Path,
    ) -> windows::core::Result<()> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let service: ITaskService =
                CoCreateInstance(&TaskSchedulerComClass, None, CLSCTX_INPROC_SERVER)?;
            service.Connect(
                &VARIANT::default(),
                &VARIANT::default(),
                &VARIANT::default(),
                &VARIANT::default(),
            )?;

            let root_folder = service.GetFolder(&BSTR::from("\\"))?;
            let task = service.NewTask(0)?;

            // RunLevel=HighestAvailable: elevated without a per-launch UAC
            // prompt, the whole point of this Scheduled Task approach
            // (design.md's Tech Decisions).
            let principal = task.Principal()?;
            principal.SetRunLevel(TASK_RUNLEVEL_HIGHEST)?;

            // Trigger: AtLogOn.
            let triggers = task.Triggers()?;
            let logon_trigger: ILogonTrigger = triggers.Create(TASK_TRIGGER_LOGON)?.cast()?;
            let _ = logon_trigger; // configured with its defaults (any user's logon)

            // Action: launch windows-core's executable.
            let actions = task.Actions()?;
            let exec_action: IExecAction = actions.Create(TASK_ACTION_EXEC)?.cast()?;
            exec_action.SetPath(&BSTR::from(executable_path.to_string_lossy().as_ref()))?;

            root_folder.RegisterTaskDefinition(
                &BSTR::from(task_name),
                &task,
                TASK_CREATE_OR_UPDATE.0,
                &VARIANT::default(),
                &VARIANT::default(),
                TASK_LOGON_INTERACTIVE_TOKEN,
                &VARIANT::default(),
            )?;

            Ok(())
        }
    }
}

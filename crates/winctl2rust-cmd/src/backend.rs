use winctl2rust_core::{WinctlError, WinctlResult};

use crate::atnow::{AtNowScheduleResult, AtNowTaskInfo, AtNowTaskRequest};
use crate::from_proc::{FromProcessRequest, FromProcessResult, ProcessSummary};
use crate::run::{LaunchProfile, LaunchResult};
use crate::script::{ScriptCommand, ScriptStepResult};

pub trait CmdBackend: Send + Sync {
    fn execute_script(&self, cmd: &ScriptCommand) -> WinctlResult<ScriptStepResult>;
    fn launch_process(&self, profile: &LaunchProfile) -> WinctlResult<LaunchResult>;
    fn list_candidate_processes(&self) -> WinctlResult<Vec<ProcessSummary>>;
    fn spawn_from_parent(&self, req: &FromProcessRequest) -> WinctlResult<FromProcessResult>;
    fn list_scheduled_tasks(&self, folder: &str) -> WinctlResult<Vec<AtNowTaskInfo>>;
    fn schedule_task(&self, req: &AtNowTaskRequest) -> WinctlResult<AtNowScheduleResult>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedCmdBackend;

impl CmdBackend for UnsupportedCmdBackend {
    fn execute_script(&self, cmd: &ScriptCommand) -> WinctlResult<ScriptStepResult> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "Command script execution ('{}') requires Windows Win32/COM APIs",
            cmd.verb_name()
        )))
    }

    fn launch_process(&self, profile: &LaunchProfile) -> WinctlResult<LaunchResult> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "Process launch ('{}') requires Windows CreateProcessW/ShellExecuteExW",
            profile.exe_path
        )))
    }

    fn list_candidate_processes(&self) -> WinctlResult<Vec<ProcessSummary>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "Process enumeration requires Windows ToolHelp32 APIs".to_string(),
        ))
    }

    fn spawn_from_parent(&self, req: &FromProcessRequest) -> WinctlResult<FromProcessResult> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "Parent-attributed launch (parent PID {}) requires Windows PROC_THREAD_ATTRIBUTE_PARENT_PROCESS",
            req.parent_pid
        )))
    }

    fn list_scheduled_tasks(&self, _folder: &str) -> WinctlResult<Vec<AtNowTaskInfo>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "Task enumeration requires Windows Task Scheduler v2 COM APIs".to_string(),
        ))
    }

    fn schedule_task(&self, req: &AtNowTaskRequest) -> WinctlResult<AtNowScheduleResult> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "Task scheduling ('{}') requires Windows Task Scheduler v2 COM APIs",
            req.task_name
        )))
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub struct MockCmdBackend {
    pub processes: std::sync::Mutex<Vec<ProcessSummary>>,
    pub tasks: std::sync::Mutex<Vec<AtNowTaskInfo>>,
    pub executed_scripts: std::sync::Mutex<Vec<String>>,
    pub launched_profiles: std::sync::Mutex<Vec<String>>,
    pub spawned_from_proc: std::sync::Mutex<Vec<(u32, String)>>,
    pub scheduled_requests: std::sync::Mutex<Vec<String>>,
    pub inject_permission_denied_on_from_proc: std::sync::atomic::AtomicBool,
}

#[cfg(any(test, feature = "test-support"))]
impl MockCmdBackend {
    pub fn with_sample_data() -> Self {
        Self {
            processes: std::sync::Mutex::new(vec![
                ProcessSummary {
                    pid: 4,
                    ppid: 0,
                    process_name: "System".to_string(),
                    threads: 180,
                    elevation_hint: "SYSTEM (requires SeDebugPrivilege/Admin)".to_string(),
                },
                ProcessSummary {
                    pid: 1024,
                    ppid: 640,
                    process_name: "explorer.exe".to_string(),
                    threads: 42,
                    elevation_hint: "Medium/CurrentUser".to_string(),
                },
            ]),
            tasks: std::sync::Mutex::new(vec![AtNowTaskInfo {
                task_name: "SampleDelayedJob".to_string(),
                folder: "\\".to_string(),
                state: "Ready".to_string(),
                enabled: true,
                next_run: Some("2026-11-23T05:00:00".to_string()),
                last_run: None,
                last_result: 0,
                command: "C:\\Windows\\System32\\cmd.exe /c echo ok".to_string(),
            }]),
            ..Self::default()
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl CmdBackend for MockCmdBackend {
    fn execute_script(&self, cmd: &ScriptCommand) -> WinctlResult<ScriptStepResult> {
        self.executed_scripts.lock().unwrap().push(cmd.verb_name().to_string());
        Ok(ScriptStepResult {
            verb: cmd.verb_name().to_string(),
            applied: true,
            detail: format!("mock executed {}", cmd.verb_name()),
        })
    }

    fn launch_process(&self, profile: &LaunchProfile) -> WinctlResult<LaunchResult> {
        self.launched_profiles.lock().unwrap().push(profile.exe_path.clone());
        Ok(LaunchResult {
            pid: Some(4242),
            command_line: profile.build_command_line(),
            run_as: format!("{:?}", profile.run_as_mode),
            exit_code: if profile.wait_for_exit { Some(0) } else { None },
        })
    }

    fn list_candidate_processes(&self) -> WinctlResult<Vec<ProcessSummary>> {
        Ok(self.processes.lock().unwrap().clone())
    }

    fn spawn_from_parent(&self, req: &FromProcessRequest) -> WinctlResult<FromProcessResult> {
        if self
            .inject_permission_denied_on_from_proc
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(WinctlError::PermissionDenied(format!(
                "mock ERROR_ACCESS_DENIED opening parent PID {}",
                req.parent_pid
            )));
        }
        self.spawned_from_proc
            .lock()
            .unwrap()
            .push((req.parent_pid, req.child_exe.clone()));
        Ok(FromProcessResult {
            parent_pid: req.parent_pid,
            child_pid: 5150,
            child_tid: 5151,
            command_line: req.build_command_line(),
        })
    }

    fn list_scheduled_tasks(&self, _folder: &str) -> WinctlResult<Vec<AtNowTaskInfo>> {
        Ok(self.tasks.lock().unwrap().clone())
    }

    fn schedule_task(&self, req: &AtNowTaskRequest) -> WinctlResult<AtNowScheduleResult> {
        self.scheduled_requests.lock().unwrap().push(req.task_name.clone());
        let boundary = req.run_at.resolve_start_boundary(chrono::Local::now())?;
        Ok(AtNowScheduleResult {
            task_name: req.task_name.clone(),
            folder: req.folder_path.clone(),
            start_boundary: boundary,
            state: "Ready".to_string(),
        })
    }
}

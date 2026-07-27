use std::convert::TryInto;
use std::io::{self, Error, ErrorKind};
use std::mem;
use std::os::windows::{io::AsRawHandle, process::CommandExt};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus};
use std::ptr;
use std::sync::Arc;

use winapi::shared::{
    minwindef::{BOOL, DWORD, FALSE, LPVOID},
    winerror::ERROR_NO_MORE_FILES,
};
use winapi::um::{
    handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
    jobapi2::{
        AssignProcessToJobObject, CreateJobObjectW, QueryInformationJobObject,
        SetInformationJobObject, TerminateJobObject,
    },
    processthreadsapi::{GetProcessId, OpenThread, ResumeThread},
    tlhelp32::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    },
    winbase::CREATE_SUSPENDED,
    winnt::{
        JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation, HANDLE,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    },
};

use super::{ProbeProcess, SpawnProbeError};

trait WindowsApi: Send + Sync {
    fn create_job(&self) -> io::Result<HANDLE>;
    fn configure_kill_on_close(&self, job: HANDLE) -> io::Result<()>;
    fn close_handle(&self, handle: HANDLE);
    fn assign_process(&self, job: HANDLE, process: HANDLE) -> io::Result<()>;
    fn resume_process(&self, process: HANDLE) -> io::Result<()>;
    fn terminate_job(&self, job: HANDLE) -> io::Result<()>;
    fn active_processes(&self, job: HANDLE) -> io::Result<DWORD>;
    fn terminate_leader(&self, child: &mut Child) -> io::Result<()>;
}

struct SystemWindowsApi;

/// Owns a valid Win32 handle from the instant an allocating API returns it.
struct OwnedHandle {
    raw: HANDLE,
    api: Arc<dyn WindowsApi>,
}

impl OwnedHandle {
    fn from_nullable(handle: HANDLE, api: Arc<dyn WindowsApi>) -> io::Result<Self> {
        if handle.is_null() {
            Err(Error::last_os_error())
        } else {
            Ok(Self { raw: handle, api })
        }
    }

    fn from_snapshot(handle: HANDLE, api: Arc<dyn WindowsApi>) -> io::Result<Self> {
        if handle == INVALID_HANDLE_VALUE {
            Err(Error::last_os_error())
        } else {
            Ok(Self { raw: handle, api })
        }
    }

    fn raw(&self) -> HANDLE {
        self.raw
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        self.api.close_handle(self.raw);
    }
}

// Win32 kernel handles may be used from the registry's cleanup supervisor.
unsafe impl Send for OwnedHandle {}

pub(crate) struct ProbeChild {
    child: Child,
    job: OwnedHandle,
    assigned: bool,
    leader_status: Option<ExitStatus>,
    group_exited: bool,
    api: Arc<dyn WindowsApi>,
}

/// A suspended process is immediately wrapped in ProbeChild. This transition
/// owner guarantees Drop rollback even before assignment or resume succeeds.
struct PendingProbeChild(ProbeChild);

impl PendingProbeChild {
    fn rollback_spawn_failure(self, error: io::Error) -> SpawnProbeError {
        SpawnProbeError::post_spawn(error, self.0)
    }

    fn assign(&mut self) -> io::Result<()> {
        self.0
            .api
            .assign_process(self.0.job.raw(), self.0.process_handle())?;
        self.0.assigned = true;
        Ok(())
    }

    fn resume(&self) -> io::Result<()> {
        self.0.api.resume_process(self.0.process_handle())
    }

    fn into_child(self) -> ProbeChild {
        self.0
    }
}

impl ProbeChild {
    fn process_handle(&self) -> HANDLE {
        self.child.as_raw_handle() as HANDLE
    }

    fn terminate_job(&self) -> io::Result<()> {
        self.api.terminate_job(self.job.raw())
    }

    fn active_processes(&self) -> io::Result<DWORD> {
        self.api.active_processes(self.job.raw())
    }

    fn spawn_with_api(
        command: &mut Command,
        api: Arc<dyn WindowsApi>,
    ) -> Result<Self, SpawnProbeError> {
        command.creation_flags(CREATE_SUSPENDED);
        let job =
            create_kill_on_close_job(Arc::clone(&api)).map_err(SpawnProbeError::before_spawn)?;
        let child = command.spawn().map_err(SpawnProbeError::before_spawn)?;
        let mut pending = PendingProbeChild(Self {
            child,
            job,
            assigned: false,
            leader_status: None,
            group_exited: false,
            api,
        });

        if let Err(error) = pending.assign() {
            return Err(pending.rollback_spawn_failure(error));
        }
        if let Err(error) = pending.resume() {
            return Err(pending.rollback_spawn_failure(error));
        }
        Ok(pending.into_child())
    }
}

impl ProbeProcess for ProbeChild {
    fn spawn(command: &mut Command) -> Result<Self, SpawnProbeError> {
        Self::spawn_with_api(command, Arc::new(SystemWindowsApi))
    }

    fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    fn try_wait_leader(&mut self) -> io::Result<bool> {
        if self.leader_status.is_none() {
            self.leader_status = self.child.try_wait()?;
        }
        Ok(self.leader_status.is_some())
    }

    fn reap_leader(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.leader_status.is_none() {
            self.leader_status = self.child.try_wait()?;
        }
        Ok(self.leader_status)
    }

    fn try_wait_group(&mut self) -> io::Result<bool> {
        if self.group_exited {
            return Ok(true);
        }
        if !self.assigned {
            self.group_exited = self.try_wait_leader()?;
        } else {
            self.group_exited = self.active_processes()? == 0;
        }
        Ok(self.group_exited)
    }

    fn kill(&mut self) -> io::Result<()> {
        let job_result = if self.assigned {
            self.terminate_job()
        } else {
            Ok(())
        };
        let child_result = self.api.terminate_leader(&mut self.child);

        match (job_result, child_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(error)) if already_exited(&error) => Ok(()),
            (Err(job_error), Ok(())) => Err(job_error),
            (Ok(()), Err(child_error)) => Err(child_error),
            (Err(job_error), Err(child_error)) if already_exited(&child_error) => Err(job_error),
            (Err(job_error), Err(child_error)) => Err(io::Error::new(
                job_error.kind(),
                format!(
                    "failed to terminate probe Job Object: {job_error}; \
                     failed to terminate probe leader: {child_error}"
                ),
            )),
        }
    }
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        // Make termination explicit before OwnedHandle closes the
        // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE backstop.
        if self.assigned {
            let _ = self.api.terminate_job(self.job.raw());
        }
        let _ = self.api.terminate_leader(&mut self.child);
        let _ = self.child.try_wait();
    }
}

fn create_kill_on_close_job(api: Arc<dyn WindowsApi>) -> io::Result<OwnedHandle> {
    let job = OwnedHandle::from_nullable(api.create_job()?, Arc::clone(&api))?;
    api.configure_kill_on_close(job.raw())?;
    Ok(job)
}

impl WindowsApi for SystemWindowsApi {
    fn create_job(&self) -> io::Result<HANDLE> {
        let handle = unsafe { CreateJobObjectW(ptr::null_mut(), ptr::null()) };
        if handle.is_null() {
            Err(Error::last_os_error())
        } else {
            Ok(handle)
        }
    }

    fn configure_kill_on_close(&self, job: HANDLE) -> io::Result<()> {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        bool_result(unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as LPVOID,
                checked_size(&info)?,
            )
        })
    }

    fn close_handle(&self, handle: HANDLE) {
        // SAFETY: OwnedHandle constructors reject the allocating API's
        // sentinel and never duplicate or transfer the handle.
        unsafe {
            CloseHandle(handle);
        }
    }

    fn assign_process(&self, job: HANDLE, process: HANDLE) -> io::Result<()> {
        bool_result(unsafe { AssignProcessToJobObject(job, process) })
    }

    fn resume_process(&self, process: HANDLE) -> io::Result<()> {
        resume_process_threads(process)
    }

    fn terminate_job(&self, job: HANDLE) -> io::Result<()> {
        bool_result(unsafe { TerminateJobObject(job, 1) })
    }

    fn active_processes(&self, job: HANDLE) -> io::Result<DWORD> {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        bool_result(unsafe {
            QueryInformationJobObject(
                job,
                JobObjectBasicAccountingInformation,
                &mut info as *mut _ as LPVOID,
                checked_size(&info)?,
                ptr::null_mut(),
            )
        })?;
        Ok(info.ActiveProcesses)
    }

    fn terminate_leader(&self, child: &mut Child) -> io::Result<()> {
        child.kill()
    }
}

fn resume_process_threads(process: HANDLE) -> io::Result<()> {
    let process_id = unsafe { GetProcessId(process) };
    if process_id == 0 {
        return Err(Error::last_os_error());
    }

    let api: Arc<dyn WindowsApi> = Arc::new(SystemWindowsApi);
    let snapshot = OwnedHandle::from_snapshot(
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) },
        Arc::clone(&api),
    )?;
    let mut entry = THREADENTRY32 {
        dwSize: checked_size::<THREADENTRY32>(&THREADENTRY32::default())?,
        ..THREADENTRY32::default()
    };
    bool_result(unsafe { Thread32First(snapshot.raw(), &mut entry) })?;

    let mut resumed = false;
    loop {
        if entry.th32OwnerProcessID == process_id {
            // THREAD_SUSPEND_RESUME
            let thread = OwnedHandle::from_nullable(
                unsafe { OpenThread(0x0002, FALSE, entry.th32ThreadID) },
                Arc::clone(&api),
            )?;
            dword_result(unsafe { ResumeThread(thread.raw()) })?;
            resumed = true;
        }

        if unsafe { Thread32Next(snapshot.raw(), &mut entry) } == FALSE {
            let error = Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                break;
            }
            return Err(error);
        }
    }

    if resumed {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::NotFound,
            "spawned probe process had no resumable thread",
        ))
    }
}

fn checked_size<T>(value: &T) -> io::Result<DWORD> {
    mem::size_of_val(value)
        .try_into()
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "Windows metadata is too large"))
}

fn bool_result(value: BOOL) -> io::Result<()> {
    if value == FALSE {
        Err(Error::last_os_error())
    } else {
        Ok(())
    }
}

fn dword_result(value: DWORD) -> io::Result<DWORD> {
    if value == DWORD::MAX {
        Err(Error::last_os_error())
    } else {
        Ok(value)
    }
}

fn already_exited(error: &io::Error) -> bool {
    matches!(error.kind(), ErrorKind::InvalidInput | ErrorKind::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use std::thread;
    use std::time::{Duration, Instant};

    static WINDOWS_PROBE_TESTS: Mutex<()> = Mutex::new(());

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum InjectedFailure {
        None,
        Assign,
        Resume,
    }

    #[derive(Default)]
    struct MockState {
        jobs_created: AtomicUsize,
        handles_closed: AtomicUsize,
        assignments: AtomicUsize,
        resumes: AtomicUsize,
        job_terminations: AtomicUsize,
        leader_terminations: AtomicUsize,
    }

    struct MockWindowsApi {
        state: Arc<MockState>,
        failure: InjectedFailure,
    }

    impl MockWindowsApi {
        fn new(state: Arc<MockState>, failure: InjectedFailure) -> Self {
            Self { state, failure }
        }
    }

    #[derive(Default)]
    struct TrackingSystemWindowsApi {
        jobs_created: AtomicUsize,
        handles_closed: AtomicUsize,
    }

    impl TrackingSystemWindowsApi {
        fn live_jobs(&self) -> usize {
            self.jobs_created.load(Ordering::SeqCst) - self.handles_closed.load(Ordering::SeqCst)
        }
    }

    impl WindowsApi for TrackingSystemWindowsApi {
        fn create_job(&self) -> io::Result<HANDLE> {
            let handle = SystemWindowsApi.create_job()?;
            self.jobs_created.fetch_add(1, Ordering::SeqCst);
            Ok(handle)
        }

        fn configure_kill_on_close(&self, job: HANDLE) -> io::Result<()> {
            SystemWindowsApi.configure_kill_on_close(job)
        }

        fn close_handle(&self, handle: HANDLE) {
            SystemWindowsApi.close_handle(handle);
            self.handles_closed.fetch_add(1, Ordering::SeqCst);
        }

        fn assign_process(&self, job: HANDLE, process: HANDLE) -> io::Result<()> {
            SystemWindowsApi.assign_process(job, process)
        }

        fn resume_process(&self, process: HANDLE) -> io::Result<()> {
            SystemWindowsApi.resume_process(process)
        }

        fn terminate_job(&self, job: HANDLE) -> io::Result<()> {
            SystemWindowsApi.terminate_job(job)
        }

        fn active_processes(&self, job: HANDLE) -> io::Result<DWORD> {
            SystemWindowsApi.active_processes(job)
        }

        fn terminate_leader(&self, child: &mut Child) -> io::Result<()> {
            SystemWindowsApi.terminate_leader(child)
        }
    }

    impl WindowsApi for MockWindowsApi {
        fn create_job(&self) -> io::Result<HANDLE> {
            self.state.jobs_created.fetch_add(1, Ordering::SeqCst);
            Ok(1_usize as HANDLE)
        }

        fn configure_kill_on_close(&self, _job: HANDLE) -> io::Result<()> {
            Ok(())
        }

        fn close_handle(&self, _handle: HANDLE) {
            self.state.handles_closed.fetch_add(1, Ordering::SeqCst);
        }

        fn assign_process(&self, _job: HANDLE, _process: HANDLE) -> io::Result<()> {
            self.state.assignments.fetch_add(1, Ordering::SeqCst);
            if self.failure == InjectedFailure::Assign {
                Err(io::Error::other("injected Job assignment failure"))
            } else {
                Ok(())
            }
        }

        fn resume_process(&self, _process: HANDLE) -> io::Result<()> {
            self.state.resumes.fetch_add(1, Ordering::SeqCst);
            if self.failure == InjectedFailure::Resume {
                Err(io::Error::other("injected process resume failure"))
            } else {
                Ok(())
            }
        }

        fn terminate_job(&self, _job: HANDLE) -> io::Result<()> {
            self.state.job_terminations.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn active_processes(&self, _job: HANDLE) -> io::Result<DWORD> {
            Ok(0)
        }

        fn terminate_leader(&self, child: &mut Child) -> io::Result<()> {
            self.state
                .leader_terminations
                .fetch_add(1, Ordering::SeqCst);
            child.kill()
        }
    }

    fn suspended_fixture_command() -> Command {
        let mut command = Command::new(std::env::current_exe().expect("current test executable"));
        command.arg("--list");
        command
    }

    #[test]
    fn missing_executable_closes_preallocated_job_handle() {
        let _serial = WINDOWS_PROBE_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = Arc::new(MockState::default());
        let api: Arc<dyn WindowsApi> = Arc::new(MockWindowsApi::new(
            Arc::clone(&state),
            InjectedFailure::None,
        ));
        let mut command = Command::new("definitely-not-a-windows-executable-xyzzy.exe");

        let result = ProbeChild::spawn_with_api(&mut command, api);
        assert!(matches!(result, Err(SpawnProbeError::BeforeSpawn(_))));
        assert_eq!(state.jobs_created.load(Ordering::SeqCst), 1);
        assert_eq!(state.handles_closed.load(Ordering::SeqCst), 1);
        assert_eq!(state.assignments.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn system_missing_executable_closes_its_job_handle() {
        let _serial = WINDOWS_PROBE_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tracking = Arc::new(TrackingSystemWindowsApi::default());
        let api: Arc<dyn WindowsApi> = tracking.clone();
        let mut command = Command::new("definitely-not-a-windows-executable-xyzzy.exe");

        let result = ProbeChild::spawn_with_api(&mut command, api);
        assert!(matches!(result, Err(SpawnProbeError::BeforeSpawn(_))));
        assert_eq!(tracking.jobs_created.load(Ordering::SeqCst), 1);
        assert_eq!(tracking.handles_closed.load(Ordering::SeqCst), 1);
        assert_eq!(tracking.live_jobs(), 0);
    }

    #[test]
    fn process_wide_baseline_misattributes_an_unrelated_live_job() {
        let _serial = WINDOWS_PROBE_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let subject = Arc::new(TrackingSystemWindowsApi::default());
        let unrelated = Arc::new(TrackingSystemWindowsApi::default());
        let process_wide_baseline = subject.live_jobs() + unrelated.live_jobs();

        let unrelated_api: Arc<dyn WindowsApi> = unrelated.clone();
        // Holding this unrelated Job open models a concurrently running probe
        // test. A process-wide counter cannot distinguish its handle from the
        // subject operation's handle.
        let unrelated_job =
            create_kill_on_close_job(unrelated_api).expect("create unrelated Job Object");

        let subject_api: Arc<dyn WindowsApi> = subject.clone();
        let mut command = Command::new("definitely-not-a-windows-executable-xyzzy.exe");
        let result = ProbeChild::spawn_with_api(&mut command, subject_api);
        assert!(matches!(result, Err(SpawnProbeError::BeforeSpawn(_))));

        assert_eq!(
            subject.live_jobs(),
            0,
            "the subject operation did not close its own Job handle"
        );
        assert_eq!(
            subject.live_jobs() + unrelated.live_jobs(),
            process_wide_baseline + 1,
            "the unrelated concurrent Job should perturb a process-wide baseline"
        );

        drop(unrelated_job);
        assert_eq!(unrelated.live_jobs(), 0);
    }

    #[test]
    fn assignment_failure_rolls_back_child_and_job_handle() {
        let _serial = WINDOWS_PROBE_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = Arc::new(MockState::default());
        let api: Arc<dyn WindowsApi> = Arc::new(MockWindowsApi::new(
            Arc::clone(&state),
            InjectedFailure::Assign,
        ));
        let mut command = suspended_fixture_command();

        let result = ProbeChild::spawn_with_api(&mut command, api);
        let child = match result {
            Err(SpawnProbeError::PostSpawn { error, child }) => {
                assert!(error.to_string().contains("assignment"), "{error}");
                child
            }
            _ => panic!("assignment failure did not return its owned child"),
        };
        drop(child);

        assert_eq!(state.assignments.load(Ordering::SeqCst), 1);
        assert_eq!(state.resumes.load(Ordering::SeqCst), 0);
        assert_eq!(state.job_terminations.load(Ordering::SeqCst), 0);
        assert_eq!(state.leader_terminations.load(Ordering::SeqCst), 1);
        assert_eq!(state.handles_closed.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn resume_failure_terminates_assigned_job_and_closes_handle() {
        let _serial = WINDOWS_PROBE_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = Arc::new(MockState::default());
        let api: Arc<dyn WindowsApi> = Arc::new(MockWindowsApi::new(
            Arc::clone(&state),
            InjectedFailure::Resume,
        ));
        let mut command = suspended_fixture_command();

        let result = ProbeChild::spawn_with_api(&mut command, api);
        let child = match result {
            Err(SpawnProbeError::PostSpawn { error, child }) => {
                assert!(error.to_string().contains("resume"), "{error}");
                child
            }
            _ => panic!("resume failure did not return its owned child"),
        };
        drop(child);

        assert_eq!(state.assignments.load(Ordering::SeqCst), 1);
        assert_eq!(state.resumes.load(Ordering::SeqCst), 1);
        assert_eq!(state.job_terminations.load(Ordering::SeqCst), 1);
        assert_eq!(state.leader_terminations.load(Ordering::SeqCst), 1);
        assert_eq!(state.handles_closed.load(Ordering::SeqCst), 1);
    }

    #[test]
    #[ignore = "helper process for drop_terminates_running_job"]
    fn windows_drop_survival_helper() {
        let started = std::env::var_os("CONJURE_WINDOWS_STARTED").expect("started marker");
        let survived = std::env::var_os("CONJURE_WINDOWS_SURVIVED").expect("survived marker");
        std::fs::write(started, b"started").expect("write started marker");
        thread::sleep(Duration::from_secs(1));
        std::fs::write(survived, b"survived").expect("write survived marker");
    }

    #[test]
    fn drop_terminates_running_job() {
        let _serial = WINDOWS_PROBE_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tracking = Arc::new(TrackingSystemWindowsApi::default());
        let api: Arc<dyn WindowsApi> = tracking.clone();
        let dir = tempfile::tempdir().expect("create marker directory");
        let started = dir.path().join("started");
        let survived = dir.path().join("survived");
        let module = module_path!()
            .strip_prefix("conjure::")
            .unwrap_or(module_path!());
        let helper = format!("{module}::windows_drop_survival_helper");
        let mut command = Command::new(std::env::current_exe().expect("current test executable"));
        command
            .args(["--exact", &helper, "--ignored", "--nocapture"])
            .env("CONJURE_WINDOWS_STARTED", &started)
            .env("CONJURE_WINDOWS_SURVIVED", &survived);
        let child = match ProbeChild::spawn_with_api(&mut command, api) {
            Ok(child) => child,
            Err(SpawnProbeError::BeforeSpawn(error)) => panic!("spawn helper: {error}"),
            Err(SpawnProbeError::PostSpawn { error, .. }) => {
                panic!("prepare helper: {error}")
            }
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while !started.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(started.exists(), "helper never reached its running state");
        assert_eq!(tracking.jobs_created.load(Ordering::SeqCst), 1);
        assert_eq!(tracking.handles_closed.load(Ordering::SeqCst), 0);

        drop(child);
        let handles_closed_deadline = Instant::now() + Duration::from_secs(1);
        while tracking.live_jobs() != 0 && Instant::now() < handles_closed_deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(tracking.handles_closed.load(Ordering::SeqCst), 1);
        assert_eq!(tracking.live_jobs(), 0);
        thread::sleep(Duration::from_millis(1_200));
        assert!(
            !survived.exists(),
            "ProbeChild::drop left its Job Object process running"
        );
    }
}

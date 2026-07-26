//! Cross-host source-shape smoke checks for the private Windows probe owner.
//! They only catch accidental architecture regressions; `cfg(windows)` unit
//! tests execute missing-binary, assignment, resume, Drop, rollback, and handle
//! cleanup behavior on Windows CI.

const WINDOWS_PROBE_CHILD: &str = include_str!("../src/commands/test/probe_child/windows.rs");
const PROBE_CHILD_API: &str = include_str!("../src/commands/test/probe_child.rs");

#[test]
fn source_shape_smoke_has_raii_handle_owner_at_each_allocator() {
    assert!(
        WINDOWS_PROBE_CHILD.contains("struct OwnedHandle")
            && WINDOWS_PROBE_CHILD.contains("impl Drop for OwnedHandle")
            && WINDOWS_PROBE_CHILD.contains("CloseHandle"),
        "every Windows HANDLE needs an immediate RAII owner"
    );
    assert!(
        WINDOWS_PROBE_CHILD.contains("CreateJobObjectW")
            && WINDOWS_PROBE_CHILD.contains("OwnedHandle::from_nullable"),
        "the Job Object must be owned immediately after allocation"
    );
    assert!(
        WINDOWS_PROBE_CHILD.contains("CreateToolhelp32Snapshot")
            && WINDOWS_PROBE_CHILD.contains("OwnedHandle::from_snapshot"),
        "the Toolhelp snapshot must be owned immediately after allocation"
    );
    assert!(
        WINDOWS_PROBE_CHILD.contains("OpenThread")
            && WINDOWS_PROBE_CHILD.contains("OwnedHandle::from_nullable"),
        "each opened thread must be owned immediately"
    );
}

#[test]
fn source_shape_smoke_keeps_suspended_child_transition_owner() {
    assert!(
        WINDOWS_PROBE_CHILD.contains("CREATE_SUSPENDED"),
        "probe spawn must suspend the child until Job assignment is complete"
    );
    assert!(
        WINDOWS_PROBE_CHILD.contains("PendingProbeChild"),
        "a suspended child needs a rollback owner before assignment or resume"
    );
    assert!(
        WINDOWS_PROBE_CHILD.contains("rollback_spawn_failure"),
        "assignment and resume failures need a shared cleanup path"
    );
    assert!(
        WINDOWS_PROBE_CHILD.contains("AssignProcessToJobObject")
            && WINDOWS_PROBE_CHILD.contains("ResumeThread"),
        "the local implementation must assign then resume the suspended process"
    );
    let assignment_failure = WINDOWS_PROBE_CHILD
        .split("if let Err(error) = pending.assign()")
        .nth(1)
        .expect("assignment failure seam");
    assert!(
        assignment_failure
            .split("if let Err(error) = pending.resume()")
            .next()
            .is_some_and(|branch| branch.contains("pending.rollback_spawn_failure(error)")),
        "assignment failure must return the suspended child to durable cleanup"
    );
    let resume_failure = WINDOWS_PROBE_CHILD
        .split("if let Err(error) = pending.resume()")
        .nth(1)
        .expect("resume failure seam");
    assert!(
        resume_failure
            .split("Ok(pending.into_child())")
            .next()
            .is_some_and(|branch| branch.contains("pending.rollback_spawn_failure(error)")),
        "resume failure must return the assigned child to durable cleanup"
    );
    assert!(
        PROBE_CHILD_API.contains("enum SpawnProbeError") && PROBE_CHILD_API.contains("PostSpawn"),
        "post-spawn failures must return the owned child for durable cleanup"
    );
    assert!(
        WINDOWS_PROBE_CHILD.contains("fn into_child(self) -> ProbeChild"),
        "the final ownership transfer must be infallible"
    );
}

#[test]
fn source_shape_smoke_allocates_job_before_fallible_spawn() {
    let job_allocation = WINDOWS_PROBE_CHILD
        .find("create_kill_on_close_job(Arc::clone(&api))")
        .expect("Job Object allocation");
    let child_spawn = WINDOWS_PROBE_CHILD
        .find("let child = command.spawn().map_err(SpawnProbeError::before_spawn)?")
        .expect("fallible child spawn");
    let pending_owner = WINDOWS_PROBE_CHILD
        .find("let mut pending = PendingProbeChild")
        .expect("suspended-child owner");
    assert!(
        job_allocation < child_spawn && child_spawn < pending_owner,
        "the RAII Job Object must exist before spawn and the child owner immediately after it"
    );
    assert!(
        WINDOWS_PROBE_CHILD.contains("JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE"),
        "closing the Job Object must be a kernel termination backstop"
    );
}

#[test]
fn source_shape_smoke_keeps_drop_and_nonblocking_job_queries() {
    assert!(
        WINDOWS_PROBE_CHILD.contains("impl Drop for ProbeChild")
            && WINDOWS_PROBE_CHILD.contains("TerminateJobObject"),
        "ProbeChild Drop must explicitly terminate its Job Object"
    );
    assert!(
        WINDOWS_PROBE_CHILD.contains("QueryInformationJobObject")
            && WINDOWS_PROBE_CHILD.contains("ActiveProcesses"),
        "group completion must be polled without a completion-port waiter"
    );
    assert!(
        WINDOWS_PROBE_CHILD.contains("if self.group_exited")
            && WINDOWS_PROBE_CHILD.contains("self.group_exited = self.active_processes()? == 0"),
        "confirmed Job completion must remain cached across repeated wait polls"
    );
    assert!(
        PROBE_CHILD_API.contains("fn try_wait_group")
            && PROBE_CHILD_API.contains("fn try_wait_leader"),
        "the cleanup supervisor needs nonblocking group and leader wait seams"
    );
}

#[test]
fn source_shape_smoke_has_no_tokio_completion_port_or_into_inner_surface() {
    for forbidden in [
        "CreateIoCompletionPort",
        "GetQueuedCompletionStatus",
        "spawn_blocking",
        "tokio",
    ] {
        assert!(
            !WINDOWS_PROBE_CHILD.contains(forbidden) && !PROBE_CHILD_API.contains(forbidden),
            "private probe implementation retained forbidden surface `{forbidden}`"
        );
    }
    assert!(
        !WINDOWS_PROBE_CHILD.contains("fn into_inner(")
            && !PROBE_CHILD_API.contains("fn into_inner("),
        "private probe implementation retained a child ownership escape hatch"
    );
}

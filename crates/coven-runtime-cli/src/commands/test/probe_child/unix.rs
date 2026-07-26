use std::io;
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus};
use std::sync::Arc;

use command_group::{CommandGroup, GroupChild};

use super::{ProbeProcess, SpawnProbeError};

pub(crate) trait UnixGroupApi: Send + Sync {
    fn group_exists(&self, process_group: i32) -> io::Result<bool>;
    fn kill_group(&self, child: &mut GroupChild) -> io::Result<()>;
}

struct SystemUnixGroupApi;

impl UnixGroupApi for SystemUnixGroupApi {
    fn group_exists(&self, process_group: i32) -> io::Result<bool> {
        let result = unsafe { libc::kill(-process_group, 0) };
        if result == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ESRCH) => Ok(false),
            _ => Err(error),
        }
    }

    fn kill_group(&self, child: &mut GroupChild) -> io::Result<()> {
        child.kill()
    }
}

pub(crate) struct ProbeChild {
    child: GroupChild,
    process_group: i32,
    leader_observed: bool,
    leader_status: Option<ExitStatus>,
    group_targetable: bool,
    post_reap_classified: bool,
    api: Arc<dyn UnixGroupApi>,
}

impl ProbeChild {
    fn spawn_with_api(
        command: &mut Command,
        api: Arc<dyn UnixGroupApi>,
    ) -> Result<Self, SpawnProbeError> {
        command
            .group_spawn()
            .map(|child| {
                // GroupChild already validated this conversion while creating
                // its Unix implementation, before it could be returned.
                let process_group = child.id() as i32;
                Self {
                    child,
                    process_group,
                    leader_observed: false,
                    leader_status: None,
                    group_targetable: true,
                    post_reap_classified: false,
                    api,
                }
            })
            .map_err(SpawnProbeError::before_spawn)
    }

    #[cfg(test)]
    pub(crate) fn spawn_with_group_api(
        command: &mut Command,
        api: Arc<dyn UnixGroupApi>,
    ) -> Result<Self, SpawnProbeError> {
        Self::spawn_with_api(command, api)
    }

    pub(crate) fn consumed_group_permission(&self, error: &io::Error) -> bool {
        !self.group_targetable && error.raw_os_error() == Some(libc::EPERM)
    }

    pub(crate) fn post_reap_group_exists(&mut self) -> io::Result<bool> {
        if self.group_targetable || self.leader_status.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "probe leader must be reaped after consuming its group target",
            ));
        }
        if self.post_reap_classified {
            return Err(io::Error::other(
                "probe process group was already classified after leader reap",
            ));
        }
        self.post_reap_classified = true;
        self.api.group_exists(self.process_group)
    }
}

impl ProbeProcess for ProbeChild {
    fn spawn(command: &mut Command) -> Result<Self, SpawnProbeError> {
        Self::spawn_with_api(command, Arc::new(SystemUnixGroupApi))
    }

    fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.inner().stdout.take()
    }

    fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.inner().stderr.take()
    }

    fn try_wait_leader(&mut self) -> io::Result<bool> {
        if !self.leader_observed {
            self.leader_observed = leader_waitable_without_reaping(self.process_group)?;
        }
        Ok(self.leader_observed)
    }

    fn reap_leader(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.group_targetable {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "probe process-group target must be consumed before reaping its leader",
            ));
        }
        if self.leader_status.is_none() {
            self.leader_status = self.child.inner().try_wait()?;
        }
        Ok(self.leader_status)
    }

    fn try_wait_group(&mut self) -> io::Result<bool> {
        if !self.group_targetable {
            return Ok(true);
        }

        // Keep the leader waitable while asking the kernel about the original
        // process group. Reaping first would allow its numeric PGID to be
        // recycled before a later killpg target.
        match self.api.group_exists(self.process_group) {
            Ok(true) => Ok(false),
            Ok(false) => {
                self.group_targetable = false;
                Ok(true)
            }
            Err(error) if error.raw_os_error() == Some(libc::EPERM) => {
                self.group_targetable = false;
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    fn kill(&mut self) -> io::Result<()> {
        if !self.group_targetable {
            return Ok(());
        }
        match self.api.kill_group(&mut self.child) {
            Ok(()) => {
                // A successful terminal signal consumes the only PGID target
                // this owner may ever use. Descendants finish asynchronously;
                // pipe readers provide the remaining owned completion signal.
                self.group_targetable = false;
                Ok(())
            }
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
                self.group_targetable = false;
                Ok(())
            }
            Err(error) if error.raw_os_error() == Some(libc::EPERM) => {
                // Permission denial permanently consumes this numeric target,
                // but remains an error until post-reap classification proves
                // it was only the anchored zombie leader.
                self.group_targetable = false;
                Err(error)
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        // GroupChild has no kill-on-drop contract on Unix. This explicit
        // process-group termination is the unwind/backstop path; normal
        // cleanup remains owned by the nonblocking cleanup supervisor.
        let _ = self.kill();
        let _ = self.reap_leader();
    }
}

fn leader_waitable_without_reaping(process_id: i32) -> io::Result<bool> {
    let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            process_id as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { info.si_pid() } != 0)
}

#[cfg(test)]
impl ProbeChild {
    pub(crate) fn leader_is_waitable_without_reaping(&self) -> io::Result<bool> {
        leader_waitable_without_reaping(self.process_group)
    }
}

impl ProbeChild {
    pub(crate) fn abandon_group_target(&mut self) {
        self.group_targetable = false;
    }
}

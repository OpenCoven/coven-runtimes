use std::io;
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus};

#[cfg(unix)]
#[path = "probe_child/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "probe_child/windows.rs"]
mod platform;

pub(super) use platform::ProbeChild;
#[cfg(all(test, unix))]
pub(crate) use platform::UnixGroupApi;

pub(super) enum SpawnProbeError {
    BeforeSpawn(io::Error),
    #[cfg(windows)]
    PostSpawn {
        error: io::Error,
        child: Box<ProbeChild>,
    },
}

impl SpawnProbeError {
    pub(super) fn before_spawn(error: io::Error) -> Self {
        Self::BeforeSpawn(error)
    }

    #[cfg(windows)]
    pub(super) fn post_spawn(error: io::Error, child: ProbeChild) -> Self {
        Self::PostSpawn {
            error,
            child: Box::new(child),
        }
    }
}

pub(super) trait ProbeProcess {
    fn spawn(command: &mut Command) -> Result<Self, SpawnProbeError>
    where
        Self: Sized;

    fn take_stdout(&mut self) -> Option<ChildStdout>;
    fn take_stderr(&mut self) -> Option<ChildStderr>;
    fn try_wait_leader(&mut self) -> io::Result<bool>;
    fn reap_leader(&mut self) -> io::Result<Option<ExitStatus>>;
    fn try_wait_group(&mut self) -> io::Result<bool>;
    fn kill(&mut self) -> io::Result<()>;
}

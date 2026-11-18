//! Starts an agent program so that it can be stopped together with everything it starts.
//!
//! An agent starts programs of its own, such as test runs and build tools.
//! Stopping the agent has to stop those too, or they keep running and holding
//! files after the thread says the agent has stopped. Each platform groups a
//! program with what it starts in its own way:
//!
//! - Linux and macOS: the program gets a process group of its own, and
//!   signals go to the whole group.
//! - Windows: the program is put in a job object, and the whole job is ended
//!   at once. Windows has no gentle interrupt for a program without a
//!   console window, so every stop ends the job immediately. The program is
//!   placed in the job right after it starts; anything it starts within that
//!   first moment escapes the job, which agent programs do not do.

use std::io::Error as IoError;

use tokio::process::{Child, Command};

/// How a program is asked to stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Ends the current turn; Claude Code stops working and exits.
    Interrupt,
    /// Asks the program to end at once, as Teryon itself shuts down.
    Terminate,
    /// Ends it without asking.
    Kill,
}

/// A running program and everything it starts.
pub struct ProcessTree {
    child: Child,
    #[cfg(unix)]
    group: Option<u32>,
    #[cfg(windows)]
    job: windows::Job,
}

impl ProcessTree {
    /// Starts `command` as the head of a tree of its own.
    pub fn start(mut command: Command) -> Result<Self, IoError> {
        command.kill_on_drop(true);
        #[cfg(unix)]
        {
            command.process_group(0);
            let child = command.spawn()?;
            let group = child.id();
            Ok(Self { child, group })
        }
        #[cfg(windows)]
        {
            let job = windows::Job::new()?;
            let child = command.spawn()?;
            if let Some(handle) = child.raw_handle() {
                job.assign(handle)?;
            }
            Ok(Self { child, job })
        }
    }

    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    /// Sends a signal to the whole tree. Does nothing when it has already ended.
    pub fn signal(&self, signal: Signal) {
        #[cfg(unix)]
        {
            use nix::sys::signal::{killpg, Signal as UnixSignal};
            use nix::unistd::Pid;

            let Some(group) = self.group.and_then(|group| i32::try_from(group).ok()) else {
                return;
            };
            let signal = match signal {
                Signal::Interrupt => UnixSignal::SIGINT,
                Signal::Terminate => UnixSignal::SIGTERM,
                Signal::Kill => UnixSignal::SIGKILL,
            };
            // The group is gone when the program has already ended; nothing to do then.
            let _ = killpg(Pid::from_raw(group), signal);
        }
        #[cfg(windows)]
        {
            let _ = signal;
            self.job.end();
        }
    }
}

#[cfg(windows)]
mod windows {
    //! A job object that ends everything in it when asked, or when Teryon goes away.

    use std::io::Error as IoError;
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::RawHandle;
    use std::ptr::null;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct Job {
        handle: HANDLE,
    }

    // The handle is only used through calls Windows makes safe from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new() -> Result<Self, IoError> {
            // SAFETY: no name and no security attributes are valid arguments.
            let handle = unsafe { CreateJobObjectW(null(), null()) };
            if handle.is_null() {
                return Err(IoError::last_os_error());
            }
            let job = Self { handle };
            // SAFETY: an all-zero structure is a valid "no limits" value.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: the pointer and size describe `limits`, which outlives the call.
            let set = unsafe {
                SetInformationJobObject(
                    job.handle,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if set == 0 {
                return Err(IoError::last_os_error());
            }
            Ok(job)
        }

        pub fn assign(&self, process: RawHandle) -> Result<(), IoError> {
            // SAFETY: both handles are open for the duration of the call.
            if unsafe { AssignProcessToJobObject(self.handle, process as HANDLE) } == 0 {
                return Err(IoError::last_os_error());
            }
            Ok(())
        }

        pub fn end(&self) {
            // SAFETY: the handle is open until the job is dropped.
            unsafe { TerminateJobObject(self.handle, 1) };
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by `new` and is closed only here.
            unsafe { CloseHandle(self.handle) };
        }
    }
}

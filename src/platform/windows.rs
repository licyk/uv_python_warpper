use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    os::windows::process::CommandExt,
    process::{Child, Command},
    sync::atomic::{AtomicBool, Ordering},
};

use windows_sys::Win32::{
    Foundation::{ERROR_INVALID_HANDLE, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler},
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Threading::{CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
    },
};

const INTERRUPTED_EXIT_CODE: u32 = 0xC000013A;
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn handle_console_event(event: u32) -> i32 {
    match event {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => {
            INTERRUPTED.store(true, Ordering::Relaxed);
            // Python receives the same console event independently. Do not forward
            // it again or terminate the wrapper before Python has finished.
            1
        }
        _ => 0,
    }
}

pub(crate) struct InterruptGuard {
    installed: bool,
}

impl InterruptGuard {
    pub(crate) fn install() -> io::Result<Self> {
        // SAFETY: The callback has the required ABI and uses only an atomic flag.
        if unsafe { SetConsoleCtrlHandler(Some(handle_console_event), 1) } == 0 {
            let error = io::Error::last_os_error();
            // A detached process may have only redirected handles and no console.
            if error.raw_os_error() != Some(ERROR_INVALID_HANDLE as i32) {
                return Err(error);
            }
            return Ok(Self { installed: false });
        }
        Ok(Self { installed: true })
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        if self.installed {
            // SAFETY: Remove the same callback installed by this guard.
            unsafe { SetConsoleCtrlHandler(Some(handle_console_event), 0) };
        }
    }
}

pub(crate) fn interrupted_exit_code() -> Option<i32> {
    INTERRUPTED
        .load(Ordering::Relaxed)
        .then_some(INTERRUPTED_EXIT_CODE as i32)
}

pub(crate) struct LookupTree(OwnedHandle);

impl LookupTree {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<(Child, Self)> {
        // SAFETY: Unnamed job with default security; returned ownership is unique.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateJobObjectW returned a valid owned handle.
        let tree = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        // SAFETY: All-zero limit information is valid; only the indicated flag is set.
        let mut limits = unsafe { std::mem::zeroed::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: The handle and initialized information buffer are valid for this call.
        if unsafe {
            SetInformationJobObject(
                tree.handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // Attach before uv can start probes, so no descendant can escape the job
        // in the interval between process creation and job assignment.
        let mut child = command.creation_flags(CREATE_SUSPENDED).spawn()?;
        if let Err(error) = tree.attach_and_resume(&child) {
            tree.terminate();
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok((child, tree))
    }

    fn handle(&self) -> HANDLE {
        self.0.as_raw_handle()
    }

    fn attach_and_resume(&self, child: &Child) -> io::Result<()> {
        // SAFETY: Both handles remain owned and valid throughout the call.
        if unsafe { AssignProcessToJobObject(self.handle(), child.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // Rust's stable Child API does not expose its initial thread handle. Since
        // the process is suspended, its only thread is safe to locate and resume.
        // SAFETY: Request a system thread snapshot; no pointer arguments are used.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: The snapshot is a valid, uniquely owned handle.
        let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        // SAFETY: The snapshot and initialized output structure are valid.
        let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
        while found != 0 {
            if entry.th32OwnerProcessID == child.id() {
                // SAFETY: Request only resume access to this child's initial thread.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: OpenThread returned a valid, uniquely owned handle.
                let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
                // SAFETY: The process was created suspended and this is its thread.
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(());
            }
            // SAFETY: The snapshot and output structure remain valid.
            found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
        }
        Err(io::Error::other("could not find uv's suspended thread"))
    }

    pub(crate) fn terminate(&self) {
        // SAFETY: The job is still owned by this object; all its probes are cancelled.
        unsafe { TerminateJobObject(self.handle(), INTERRUPTED_EXIT_CODE) };
    }
}

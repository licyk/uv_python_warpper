use std::{
    io,
    os::unix::process::CommandExt,
    process::{Child, Command},
    sync::atomic::{AtomicI32, Ordering},
};

static INTERRUPTED: AtomicI32 = AtomicI32::new(0);

extern "C" fn handle_signal(signal: libc::c_int) {
    // Only async-signal-safe atomic operations are used in the handler.
    let _ = INTERRUPTED.compare_exchange(0, signal, Ordering::Relaxed, Ordering::Relaxed);
}

pub(crate) struct InterruptGuard(Vec<(libc::c_int, libc::sigaction)>);

impl InterruptGuard {
    pub(crate) fn install() -> io::Result<Self> {
        let mut guard = Self(Vec::new());
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            // SAFETY: Both sigaction structures are initialized, and their pointers
            // remain valid for each call. The handler only writes an atomic flag.
            unsafe {
                let mut old = std::mem::zeroed::<libc::sigaction>();
                if libc::sigaction(signal, std::ptr::null(), &mut old) != 0 {
                    return Err(io::Error::last_os_error());
                }
                // Preserve shell job-control semantics for inherited ignored signals.
                if old.sa_sigaction == libc::SIG_IGN {
                    continue;
                }
                let mut action = std::mem::zeroed::<libc::sigaction>();
                action.sa_sigaction = handle_signal as *const () as libc::sighandler_t;
                action.sa_flags = libc::SA_RESTART;
                libc::sigemptyset(&mut action.sa_mask);
                if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                    return Err(io::Error::last_os_error());
                }
                guard.0.push((signal, old));
            }
        }
        Ok(guard)
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        for (signal, old) in self.0.iter().rev() {
            // SAFETY: Restore the disposition saved for this signal on installation.
            unsafe { libc::sigaction(*signal, old, std::ptr::null_mut()) };
        }
    }
}

pub(crate) fn interrupted_exit_code() -> Option<i32> {
    match INTERRUPTED.load(Ordering::Relaxed) {
        0 => None,
        signal => Some(128 + signal),
    }
}

pub(crate) struct LookupTree(libc::pid_t);

impl LookupTree {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<(Child, Self)> {
        // uv's probes share this group. Terminal signals reach the wrapper, which
        // cancels the whole lookup, rather than racing uv's own signal handler.
        let child = command.process_group(0).spawn()?;
        let tree = Self(child.id() as libc::pid_t);
        Ok((child, tree))
    }

    pub(crate) fn terminate(&self) {
        // SAFETY: The negative, nonzero child PID selects only its process group.
        unsafe { libc::kill(-self.0, libc::SIGKILL) };
    }
}

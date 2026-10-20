use std::{
    error::Error,
    fmt, io,
    io::Read,
    process::{Child, Command, Output},
    sync::mpsc,
    thread,
    time::Duration,
};

use crate::platform;

#[derive(Debug)]
pub(crate) struct Interrupted(pub(crate) i32);

impl fmt::Display for Interrupted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Python lookup interrupted (exit code {})", self.0)
    }
}

impl Error for Interrupted {}

pub(crate) fn check_interrupted() -> Result<(), Interrupted> {
    match platform::interrupted_exit_code() {
        Some(code) => Err(Interrupted(code)),
        None => Ok(()),
    }
}

struct LookupChild {
    child: Child,
    tree: platform::LookupTree,
    complete: bool,
}

impl Drop for LookupChild {
    fn drop(&mut self) {
        if !self.complete {
            // Also clean up Python probes that uv may have started.
            self.tree.terminate();
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub(crate) fn output(command: &mut Command) -> Result<Output, Box<dyn Error>> {
    check_interrupted()?;
    let (child, tree) = platform::LookupTree::spawn(command)?;
    let mut process = LookupChild {
        child,
        tree,
        complete: false,
    };
    let mut stdout = process
        .child
        .stdout
        .take()
        .expect("uv stdout must be piped");
    let (sender, receiver) = mpsc::channel();
    // Drain output concurrently: a full pipe must not prevent cancellation.
    thread::Builder::new()
        .name("uv-stdout".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout.read_to_end(&mut bytes).map(|_| bytes);
            let _ = sender.send(result);
        })?;

    let mut status = None;
    let mut stdout = None;
    loop {
        check_interrupted()?;
        if status.is_none() {
            status = process.child.try_wait()?;
        }
        if stdout.is_none() {
            match receiver.try_recv() {
                Ok(result) => stdout = Some(result?),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(io::Error::other("uv stdout reader stopped unexpectedly").into());
                }
            }
        }
        if let (Some(status), Some(stdout)) = (status, stdout.as_mut()) {
            check_interrupted()?;
            process.complete = true;
            return Ok(Output {
                status,
                stdout: std::mem::take(stdout),
                stderr: Vec::new(),
            });
        }
        thread::sleep(Duration::from_millis(10));
    }
}

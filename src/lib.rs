use std::{
    env,
    ffi::OsString,
    process::{Command, ExitStatus, Stdio},
};

#[derive(Clone, Copy)]
pub enum Program {
    Python,
    Pip,
}

pub fn main_for(program: Program) {
    match run(program) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}

fn run(program: Program) -> Result<i32, Box<dyn std::error::Error>> {
    let python = find_python()?;
    let args = command_args(program, env::args_os().skip(1));

    #[cfg(unix)]
    {
        run_unix(python, args)
    }

    #[cfg(windows)]
    {
        run_windows(python, args)
    }
}

fn find_python() -> Result<OsString, Box<dyn std::error::Error>> {
    let output = Command::new("uv")
        .args(["python", "find", "--system"])
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "`uv python find --system` failed (exit code {:?})",
            output.status.code()
        )
        .into());
    }

    let mut stdout = String::from_utf8(output.stdout)?;

    while stdout.ends_with('\n') || stdout.ends_with('\r') {
        stdout.pop();
    }

    if stdout.is_empty() {
        return Err("uv returned an empty python path".into());
    }

    Ok(OsString::from(stdout))
}

fn command_args(program: Program, args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut command_args = Vec::new();

    if matches!(program, Program::Pip) {
        command_args.extend([OsString::from("-m"), OsString::from("pip")]);
    }

    command_args.extend(args);
    command_args
}

#[cfg(unix)]
fn run_unix(python: OsString, args: Vec<OsString>) -> Result<i32, Box<dyn std::error::Error>> {
    use std::os::unix::process::CommandExt;

    let mut cmd = Command::new(python);

    cmd.args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let err = cmd.exec();

    Err(Box::new(err))
}

#[cfg(windows)]
fn run_windows(python: OsString, args: Vec<OsString>) -> Result<i32, Box<dyn std::error::Error>> {
    let status = Command::new(python)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    Ok(exit_code(status))
}

#[cfg(windows)]
fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::{Program, command_args};
    use std::ffi::OsString;

    #[test]
    fn python_forwards_user_arguments_unchanged() {
        assert_eq!(
            command_args(
                Program::Python,
                [OsString::from("-c"), OsString::from("print('hello')")],
            ),
            [OsString::from("-c"), OsString::from("print('hello')")],
        );
    }

    #[test]
    fn pip_runs_as_a_python_module() {
        assert_eq!(
            command_args(
                Program::Pip,
                [OsString::from("install"), OsString::from("ruff")]
            ),
            [
                OsString::from("-m"),
                OsString::from("pip"),
                OsString::from("install"),
                OsString::from("ruff"),
            ],
        );
    }
}

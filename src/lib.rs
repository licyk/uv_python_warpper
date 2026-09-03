use std::{
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

const REENTRY_GUARD_ENV: &str = "UV_PYTHON_WRAPPER_ACTIVE";

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
    if let Some(active_wrapper) = env::var_os(REENTRY_GUARD_ENV) {
        return Err(format!(
            "recursive Python wrapper invocation detected (active wrapper: {})",
            active_wrapper.to_string_lossy()
        )
        .into());
    }

    let current_exe = env::current_exe()?;
    let python = find_python(&current_exe)?;
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

fn find_python(current_exe: &Path) -> Result<OsString, Box<dyn std::error::Error>> {
    let path = env::var_os("PATH").ok_or("PATH is not set")?;
    let uv = find_uv_on_path(&path).ok_or("`uv` was not found on PATH")?;
    let wrapper_dir = current_exe
        .parent()
        .ok_or("failed to determine the wrapper directory")?;
    let python_search_path = exclude_directory_from_path(&path, wrapper_dir)?;

    let output = Command::new(uv)
        .args(["python", "find", "--system"])
        .env("PATH", python_search_path)
        .env(REENTRY_GUARD_ENV, current_exe)
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

    let python = PathBuf::from(stdout);
    validate_python_path(&python, current_exe)?;

    Ok(python.into_os_string())
}

fn find_uv_on_path(path: &OsStr) -> Option<PathBuf> {
    env::split_paths(path)
        .map(|directory| directory.join(uv_executable_name()))
        .find(|candidate| candidate.is_file())
}

fn uv_executable_name() -> &'static str {
    if cfg!(windows) { "uv.exe" } else { "uv" }
}

fn python_executable_name() -> &'static str {
    if cfg!(windows) {
        "python.exe"
    } else {
        "python"
    }
}

fn exclude_directory_from_path(
    path: &OsStr,
    excluded_directory: &Path,
) -> Result<OsString, env::JoinPathsError> {
    env::join_paths(
        env::split_paths(path)
            .filter(|directory| !paths_refer_to_same_location(directory, excluded_directory)),
    )
}

fn validate_python_path(
    python: &Path,
    current_exe: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let wrapper_python = current_exe
        .parent()
        .ok_or("failed to determine the wrapper directory")?
        .join(python_executable_name());

    if paths_refer_to_same_location(python, current_exe)
        || paths_refer_to_same_location(python, &wrapper_python)
    {
        return Err(format!(
            "`uv python find --system` resolved back to the wrapper: {}",
            python.display()
        )
        .into());
    }

    Ok(())
}

fn paths_refer_to_same_location(left: &Path, right: &Path) -> bool {
    let left = normalize_path(left);
    let right = normalize_path(right);

    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }

    #[cfg(not(windows))]
    {
        left == right
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()
                .map(|current_dir| current_dir.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
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
    use super::{
        Program, command_args, exclude_directory_from_path, python_executable_name,
        validate_python_path,
    };
    use std::{env, ffi::OsString, path::PathBuf};

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

    #[test]
    fn wrapper_directory_is_excluded_from_python_search_path() {
        let wrapper_dir = env::current_dir().unwrap();
        let other_dir = wrapper_dir.parent().unwrap().to_path_buf();
        let path = env::join_paths([wrapper_dir.clone(), other_dir.clone()]).unwrap();

        let filtered = exclude_directory_from_path(&path, &wrapper_dir).unwrap();

        assert_eq!(env::split_paths(&filtered).collect::<Vec<_>>(), [other_dir]);
    }

    #[test]
    fn discovered_current_wrapper_is_rejected() {
        let current_exe = env::current_exe().unwrap();

        assert!(validate_python_path(&current_exe, &current_exe).is_err());
    }

    #[test]
    fn discovered_sibling_python_wrapper_is_rejected() {
        let wrapper_dir = env::current_dir().unwrap().join("wrapper");
        let current_exe = wrapper_dir.join(if cfg!(windows) { "pip.exe" } else { "pip" });
        let python = wrapper_dir.join(python_executable_name());

        assert!(validate_python_path(&python, &current_exe).is_err());
    }

    #[test]
    fn unrelated_python_is_accepted() {
        let current_exe = PathBuf::from("wrapper").join(if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        });
        let python = PathBuf::from("runtime").join(python_executable_name());

        assert!(validate_python_path(&python, &current_exe).is_ok());
    }
}

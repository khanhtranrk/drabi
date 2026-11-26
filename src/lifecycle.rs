use crate::config::{Paths, atomic_write};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{self, File},
    os::fd::AsRawFd,
    os::unix::process::CommandExt,
    process::{Command, ExitStatus, Stdio},
};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Lifecycle {
    pub detached: bool,
    pub track_playtime: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pre_launch: Vec<String>,
    pub post_exit: Vec<String>,
}

impl Lifecycle {
    pub fn validate_mode(&self, detached: bool) -> Result<()> {
        anyhow::ensure!(
            !detached || self.is_empty_except_mode(),
            "Playtime and lifecycle hooks require a managed launch. Use --managed"
        );
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        !self.detached && self.is_empty_except_mode()
    }

    fn is_empty_except_mode(&self) -> bool {
        !self.track_playtime && self.pre_launch.is_empty() && self.post_exit.is_empty()
    }

    pub fn validate_hooks(&self) -> Result<()> {
        for (name, argv) in [
            ("pre_launch", &self.pre_launch),
            ("post_exit", &self.post_exit),
        ] {
            if let Some(program) = argv.first() {
                anyhow::ensure!(
                    !program.is_empty() && argv.iter().all(|arg| !arg.contains('\0')),
                    "Invalid {name} argv. Provide a non-empty executable and arguments without NUL bytes"
                );
            }
        }
        Ok(())
    }
}

pub fn playtime(paths: &Paths, id: &str) -> Result<u64> {
    paths.game_path(id)?;
    match fs::read_to_string(paths.data.join("playtime").join(id)) {
        Ok(raw) => raw.trim().parse().context("Invalid saved playtime"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

pub fn record(paths: &Paths, id: &str, elapsed: u64) -> Result<()> {
    let _lock = paths.lock_config()?;
    let total = playtime(paths, id)?.saturating_add(elapsed);
    atomic_write(
        &paths.data.join("playtime").join(id),
        &format!("{total}\n"),
        true,
    )
}

pub fn last_played(paths: &Paths, id: &str) -> Option<u64> {
    paths.game_path(id).ok()?;
    fs::read_to_string(paths.data.join("recent").join(id))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub fn note_launch(paths: &Paths, id: Option<&str>) {
    if let Some(id) = id
        && let Err(error) = write_recent(paths, id)
    {
        eprintln!("warning: could not record recent game: {error:#}");
    }
}

fn write_recent(paths: &Paths, id: &str) -> Result<()> {
    let _lock = paths.lock_config()?;
    if paths.game_path(id)?.is_file() {
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
        atomic_write(
            &paths.data.join("recent").join(id),
            &time.as_millis().to_string(),
            true,
        )?;
    }
    Ok(())
}

// Call under the configuration lock after deleting the registration.
pub fn forget_recent(paths: &Paths, id: &str) {
    if let Err(error) = fs::remove_file(paths.data.join("recent").join(id))
        && error.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("warning: could not remove recent history for {id:?}: {error}");
    }
}

pub fn detach(paths: &Paths, mut command: Command, lease: Option<File>) -> Result<i32> {
    let lease_fd = lease.as_ref().map(AsRawFd::as_raw_fd);
    if let Some(fd) = lease_fd {
        let mut supervisor = Command::new(std::env::current_exe()?);
        supervisor
            .arg("__wait-runner")
            .arg(fd.to_string())
            .arg("--")
            .arg(command.get_program())
            .args(command.get_args());
        if let Some(directory) = command.get_current_dir() {
            supervisor.current_dir(directory);
        }
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => {
                    supervisor.env(key, value);
                }
                None => {
                    supervisor.env_remove(key);
                }
            }
        }
        command = supervisor;
    }
    let directory = paths.data.join("logs");
    fs::create_dir_all(&directory)?;
    let log = tempfile::Builder::new()
        .prefix("launch-")
        .suffix(".log")
        .tempfile_in(directory)?;
    let stderr = log.as_file().try_clone()?;
    command
        .stdin(Stdio::null())
        .stdout(log.as_file().try_clone()?)
        .stderr(stderr);
    // setsid and fcntl are async-signal-safe. Only the supervisor inherits the lease.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if let Some(fd) = lease_fd
                && libc::fcntl(fd, libc::F_SETFD, 0) == -1
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let (_, path) = log.keep()?;
    match command.spawn() {
        Ok(child) => {
            println!("Started PID {}. Log at {}", child.id(), path.display());
            Ok(0)
        }
        Err(error) => {
            let _ = fs::remove_file(path);
            Err(error).context("Starting a detached launch")
        }
    }
}

pub fn wait_runner(fd: i32, argv: &[OsString]) -> Result<i32> {
    let (program, args) = argv.split_first().context("Missing program")?;
    // The supervisor keeps the inherited lease while waiting. The game gets no descriptor.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        return Err(std::io::Error::last_os_error())
            .context("Protecting the inherited runner lease");
    }
    let mut command = Command::new(program);
    command.args(args);
    crate::launch::exit_code(command)
}

pub fn run_hook(
    name: &str,
    argv: &[String],
    game: &Command,
    exit_code: Option<i32>,
) -> Result<Option<ExitStatus>> {
    let Some((program, arguments)) = argv.split_first() else {
        return Ok(None);
    };
    let mut hook = Command::new(program);
    hook.args(arguments);
    if let Some(directory) = game.get_current_dir() {
        hook.current_dir(directory);
    }
    for (key, value) in game.get_envs() {
        match value {
            Some(value) => hook.env(key, value),
            None => hook.env_remove(key),
        };
    }
    match exit_code {
        Some(code) => hook.env("DRABI_EXIT_CODE", code.to_string()),
        None => hook.env_remove("DRABI_EXIT_CODE"),
    };
    // Inherit stderr so hook diagnostics reach the same destination as game diagnostics.
    hook.status()
        .with_context(|| format!("Starting the {name} hook ({program:?})"))
        .map(Some)
}

pub fn post_exit(lifecycle: &Lifecycle, game: &Command, code: i32) {
    match run_hook("post_exit", &lifecycle.post_exit, game, Some(code)) {
        Ok(Some(status)) if !status.success() => {
            eprintln!("warning: post_exit hook exited with {status}");
        }
        Err(error) => eprintln!("warning: {error:#}"),
        _ => {}
    }
}

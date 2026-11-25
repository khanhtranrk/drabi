use crate::config::{Paths, atomic_write};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{self, File},
    os::fd::AsRawFd,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Lifecycle {
    pub detached: bool,
    pub track_playtime: bool,
    pub post_exit: Vec<String>,
}

impl Lifecycle {
    pub fn validate_mode(&self, detached: bool) -> Result<()> {
        anyhow::ensure!(
            !detached || (!self.track_playtime && self.post_exit.is_empty()),
            "Playtime and post-exit commands require a managed launch. Use --managed"
        );
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        !self.detached && !self.track_playtime && self.post_exit.is_empty()
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

pub fn post_exit(argv: &[String], directory: &std::path::Path, code: i32) -> Result<()> {
    if let Some((program, arguments)) = argv.split_first() {
        let status = Command::new(program)
            .args(arguments)
            .current_dir(directory)
            .env("DRABI_EXIT_CODE", code.to_string())
            .status()
            .context("Starting the post-exit command")?;
        anyhow::ensure!(status.success(), "Post-exit command exited with {status}");
    }
    Ok(())
}

use crate::config::{Paths, atomic_write};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
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

pub fn detach(paths: &Paths, mut command: Command) -> Result<i32> {
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
    // setsid is async-signal-safe and gives the child its own session.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
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

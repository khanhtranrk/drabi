use crate::{
    config::{Paths, atomic_write},
    launch, runner, runtime,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Default, Deserialize, Serialize)]
struct Registry {
    paths: BTreeSet<PathBuf>,
}

fn read(paths: &Paths) -> Result<Registry> {
    match fs::read_to_string(paths.config.join("prefixes.toml")) {
        Ok(raw) => Ok(toml::from_str(&raw)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn remember(paths: &Paths, prefix: &Path) -> Result<()> {
    let _lock = paths.lock_config()?;
    let mut known = read(paths)?;
    if known.paths.insert(prefix.to_owned()) {
        atomic_write(
            &paths.config.join("prefixes.toml"),
            &toml::to_string_pretty(&known)?,
            true,
        )?;
    }
    Ok(())
}

pub fn list(paths: &Paths) -> Result<()> {
    let mut known = read(paths)?.paths;
    known.insert(launch::prefix_path(paths, None)?);
    for id in crate::game_ids(paths)? {
        let game = paths.load_game(&id)?;
        known.insert(launch::prefix_path(
            paths,
            game.prefix.as_ref().map(|prefix| prefix.path.as_path()),
        )?);
    }
    for path in known {
        let state = if launch::initialized(&path) {
            "initialized"
        } else {
            "missing or uninitialized"
        };
        println!("{}\t{state}", path.display());
    }
    Ok(())
}

pub fn tool(
    paths: &Paths,
    prefix: &Path,
    selection: Option<&str>,
    tool: &str,
    verbs: &[String],
) -> Result<i32> {
    let prefix = launch::prefix_path(paths, Some(prefix))?;
    launch::check_prefix(&prefix)?;
    ensure!(
        verbs
            .iter()
            .all(|verb| !verb.starts_with('-') && !verb.is_empty()),
        "Pass Winetricks verb names"
    );
    let proton = runner::resolve(paths, selection)?;
    let umu = runtime::ensure_installed(paths, false)?;
    launch::check_prefix(&prefix)?;
    let mut command = launch::umu_command(&umu, &prefix, &proton);
    command.arg(tool).args(verbs);
    launch::exit_code(command)
}

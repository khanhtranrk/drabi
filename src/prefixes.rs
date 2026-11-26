use crate::{
    config::{Paths, atomic_write},
    launch, output, releases, runner, runtime,
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

pub fn list(paths: &Paths, format: output::Format) -> Result<()> {
    let mut known = read(paths)?.paths;
    known.insert(launch::prefix_path(paths, None)?);
    for id in crate::game_ids(paths)? {
        let game = paths.load_game(&id)?;
        known.insert(launch::prefix_path(
            paths,
            game.prefix.as_ref().map(|prefix| prefix.path.as_path()),
        )?);
    }
    let rows: Vec<_> = known
        .into_iter()
        .map(|path| match launch::initialized(&path) {
            Ok(initialized) => PrefixRow {
                path,
                status: if initialized {
                    "initialized"
                } else {
                    "missing_or_uninitialized"
                },
                error: None,
            },
            Err(error) => PrefixRow {
                path,
                status: "invalid",
                error: Some(format!("{error:#}")),
            },
        })
        .collect();
    output::list(
        format,
        ["Prefix", "Status"],
        &rows,
        |row| {
            [
                row.path.display().to_string(),
                row.error.as_ref().map_or_else(
                    || row.status.replace('_', " "),
                    |error| format!("invalid: {error}"),
                ),
            ]
        },
        "No known prefixes.",
    )
}

#[derive(Serialize)]
struct PrefixRow {
    path: PathBuf,
    status: &'static str,
    error: Option<String>,
}

pub fn inspect(paths: &Paths, path: Option<&Path>, format: output::Format) -> Result<()> {
    let path = launch::prefix_path(paths, path)?;
    launch::check_prefix(&path)?;
    if matches!(format, output::Format::Json) {
        output::json(&PrefixRow {
            path,
            status: "initialized",
            error: None,
        })?;
    } else {
        println!("Initialized prefix: {}", path.display());
    }
    Ok(())
}

pub fn winetricks(
    paths: &Paths,
    prefix: Option<&Path>,
    selection: Option<&str>,
    arguments: &[String],
) -> Result<i32> {
    let (prefix, verbs) = match (prefix, arguments.split_first()) {
        (None, Some((first, rest)))
            if first.contains('/')
                || first == "~"
                || (!rest.is_empty() && Path::new(first).is_dir()) =>
        {
            (Some(Path::new(first)), rest)
        }
        _ => (prefix, arguments),
    };
    ensure!(!verbs.is_empty(), "Pass at least one Winetricks verb");
    tool(paths, prefix, selection, "winetricks", verbs)
}

pub fn tool(
    paths: &Paths,
    prefix: Option<&Path>,
    selection: Option<&str>,
    tool: &str,
    verbs: &[String],
) -> Result<i32> {
    let prefix = launch::prefix_path(paths, prefix)?;
    launch::check_prefix(&prefix)?;
    ensure!(
        verbs
            .iter()
            .all(|verb| !verb.starts_with('-') && !verb.is_empty()),
        "Pass Winetricks verb names"
    );
    let proton = runner::resolve(paths, selection)?;
    let umu = runtime::ensure_installed(paths)?;
    launch::check_prefix(&prefix)?;
    let _runner_lease = releases::lease(paths, &proton)?;
    if verbs.is_empty() {
        let mut command = launch::umu_command(&umu, &prefix, &proton);
        command.arg(tool);
        return launch::exit_code(command);
    }

    // UMU can skip the whole invocation if any supplied verb is already installed.
    let mut first_failure = 0;
    for verb in verbs {
        launch::check_prefix(&prefix)?;
        let mut command = launch::umu_command(&umu, &prefix, &proton);
        command.arg(tool).arg(verb);
        let code = launch::exit_code(command)?;
        if code != 0 {
            eprintln!("warning: Winetricks verb {verb:?} exited with status {code}");
            if first_failure == 0 {
                first_failure = code;
            }
        }
    }
    Ok(first_failure)
}

use crate::config::{Paths, Settings, atomic_write, validate_id};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub fn executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

fn validate(path: &Path) -> Result<PathBuf> {
    ensure!(
        executable(&path.join("proton")),
        "Missing executable proton script in {}",
        path.display()
    );
    Ok(path.canonicalize()?)
}

pub fn scan(paths: &Paths) -> Result<BTreeMap<String, PathBuf>> {
    let root = paths.data.join("runners/proton");
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.into()),
    };
    let mut runners = BTreeMap::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && let Some(name) = entry.file_name().to_str()
            && let Ok(path) = validate(&entry.path())
        {
            runners.insert(name.to_string(), path);
        }
    }
    Ok(runners)
}

pub fn resolve(paths: &Paths, requested: Option<&str>) -> Result<PathBuf> {
    let settings = paths.settings()?;
    let managed = scan(paths)?;
    if let Some(name) = requested.or(settings.default_runner.as_deref()) {
        if name.contains('/') {
            return validate(&paths.absolute(Path::new(name))?);
        }
        if let Some(path) = settings.external_runners.get(name) {
            return validate(&paths.absolute(path)?);
        }
        return managed
            .get(name)
            .cloned()
            .or_else(|| {
                crate::releases::steam_runner(paths, name).and_then(|path| path.canonicalize().ok())
            })
            .with_context(|| format!("Unknown Proton runner {name:?}. Run drabi proton list"));
    }

    match (settings.external_runners.len(), managed.len()) {
        (1, 0) => validate(&paths.absolute(settings.external_runners.values().next().unwrap())?),
        (0, 1) => Ok(managed.into_values().next().unwrap()),
        (0, 0) => {
            let steam = crate::releases::steam_runners(paths).collect::<Vec<_>>();
            match steam.as_slice() {
                [(_, path)] => validate(path),
                [] => bail!("No Proton runner found. Use drabi proton install latest"),
                _ => bail!(
                    "Multiple Steam runners found. Use --runner or set default_runner in config.toml"
                ),
            }
        }
        _ => bail!(
            "Multiple Proton runners found. Use --runner or set default_runner in config.toml"
        ),
    }
}

pub fn update_registration(paths: &Paths, name: &str, path: Option<&Path>) -> Result<()> {
    validate_id(name)?;
    let _lock = paths.lock_config()?;
    let mut settings: Settings = paths.settings()?;
    if let Some(path) = path {
        ensure!(
            !scan(paths)?.contains_key(name),
            "A discovered runner already uses {name:?}"
        );
        ensure!(
            !settings.external_runners.contains_key(name),
            "Runner {name:?} is already registered"
        );
        settings
            .external_runners
            .insert(name.to_owned(), validate(&paths.absolute(path)?)?);
    } else {
        ensure!(
            settings.external_runners.remove(name).is_some(),
            "Unknown external runner {name:?}"
        );
        if settings.default_runner.as_deref() == Some(name) {
            settings.default_runner = None;
        }
    }
    atomic_write(
        &paths.config.join("config.toml"),
        &toml::to_string_pretty(&settings)?,
        true,
    )
}

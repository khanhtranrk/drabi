use anyhow::{Context, Result, bail, ensure};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub home: PathBuf,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub default_runner: Option<String>,
    pub external_runners: BTreeMap<String, PathBuf>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameFile {
    pub game: Game,
    pub runner: Option<Runner>,
    pub prefix: Option<Prefix>,
    #[serde(default)]
    pub umu: Umu,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Desktop::is_empty")]
    pub desktop: Desktop,
    #[serde(
        default,
        skip_serializing_if = "crate::performance::Performance::is_empty"
    )]
    pub performance: crate::performance::Performance,
    #[serde(
        default,
        skip_serializing_if = "crate::performance::Gamescope::is_empty"
    )]
    pub gamescope: crate::performance::Gamescope,
    #[serde(default, skip_serializing_if = "crate::lifecycle::Lifecycle::is_empty")]
    pub lifecycle: crate::lifecycle::Lifecycle,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub exe: PathBuf,
    pub working_dir: Option<PathBuf>,
    #[serde(default)]
    pub arguments: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Runner {
    pub name: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Prefix {
    pub path: PathBuf,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Umu {
    pub game_id: Option<String>,
    pub store: Option<String>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Desktop {
    pub name: Option<String>,
    pub icon: Option<PathBuf>,
}

impl Desktop {
    fn is_empty(&self) -> bool {
        self.name.is_none() && self.icon.is_none()
    }
}

impl Paths {
    pub fn load() -> Result<Self> {
        let dirs = BaseDirs::new().context("Cannot determine home directory")?;
        Ok(Self {
            config: dirs.config_dir().join("drabi"),
            data: dirs.data_dir().join("drabi"),
            home: dirs.home_dir().to_owned(),
        })
    }

    pub fn absolute(&self, path: &Path) -> Result<PathBuf> {
        let path = match path.strip_prefix("~") {
            Ok(rest) => self.home.join(rest),
            Err(_) => path.to_owned(),
        };
        Ok(std::path::absolute(path)?)
    }

    pub fn game_path(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        Ok(self.config.join("games").join(format!("{id}.toml")))
    }

    pub fn lock_config(&self) -> Result<File> {
        fs::create_dir_all(&self.config)?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.config.join(".config.lock"))?;
        lock.lock()?;
        Ok(lock)
    }

    pub fn settings(&self) -> Result<Settings> {
        match fs::read_to_string(self.config.join("config.toml")) {
            Ok(raw) => toml::from_str(&raw).context("Invalid config.toml"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn load_game(&self, id: &str) -> Result<GameFile> {
        let path = self.game_path(id)?;
        let raw =
            fs::read_to_string(&path).with_context(|| format!("Reading {}", path.display()))?;
        parse_game(&raw, id)
    }
}

pub fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 80
            && id
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_')),
        "Invalid ID {id:?}. Use 1–80 ASCII letters, digits, hyphens or underscores"
    );
    Ok(())
}

pub fn parse_game(raw: &str, id: &str) -> Result<GameFile> {
    let file: GameFile = toml::from_str(raw).with_context(|| format!("Invalid TOML for {id}"))?;
    ensure!(file.game.id == id, "Game ID must match the filename ({id})");
    ensure!(!file.game.name.trim().is_empty(), "Game name is required");
    for path in std::iter::once(&file.game.exe)
        .chain(file.game.working_dir.iter())
        .chain(file.prefix.iter().map(|prefix| &prefix.path))
    {
        ensure!(
            path.is_absolute() || path.starts_with("~"),
            "Expected an absolute or ~/ path, got {}",
            path.display()
        );
    }
    for (key, value) in &file.environment {
        validate_env(key, value)?;
    }
    Ok(file)
}

pub fn validate_env(key: &str, value: &str) -> Result<()> {
    ensure!(
        !key.is_empty() && !key.contains(['=', '\0']) && !value.contains('\0'),
        "Invalid environment entry for {key:?}"
    );
    if key.starts_with("UMU_")
        || matches!(
            key,
            "PROTONPATH"
                | "WINEPREFIX"
                | "GAMEID"
                | "STORE"
                | "PROTON_VERB"
                | "STEAM_COMPAT_DATA_PATH"
                | "STEAM_COMPAT_INSTALL_PATH"
        )
    {
        bail!("{key} is managed by drabi. Use the corresponding launch option");
    }
    Ok(())
}

pub fn atomic_write(path: &Path, body: &str, replace: bool) -> Result<()> {
    let directory = path.parent().context("Path has no parent")?;
    fs::create_dir_all(directory)?;
    let mut staged = tempfile::NamedTempFile::new_in(directory)?;
    staged.write_all(body.as_bytes())?;
    staged.as_file().sync_all()?;
    if replace {
        staged.persist(path)?;
    } else {
        staged.persist_noclobber(path)?;
    }
    Ok(())
}

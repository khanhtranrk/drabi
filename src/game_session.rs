use crate::{config::Paths, launch, lifecycle};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::Write,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicI32, Ordering},
};

static STOP: AtomicI32 = AtomicI32::new(0);

extern "C" fn request_stop(signal: libc::c_int) {
    STOP.store(signal, Ordering::Relaxed);
}

fn handle_shutdown() -> Result<()> {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // The handler only stores the signal number in a lock-free atomic.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = request_stop as *const () as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } == -1 {
            return Err(std::io::Error::last_os_error()).context("Installing shutdown handler");
        }
    }
    Ok(())
}

fn lease_path(paths: &Paths) -> std::path::PathBuf {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute());
    let mut hash = Sha256::new();
    hash.update(paths.config.as_os_str().as_bytes());
    if let Some(display) = env::var_os("WAYLAND_DISPLAY") {
        hash.update([0]);
        hash.update(display.as_bytes());
        let socket = runtime
            .as_ref()
            .map(|path| path.join(&display))
            .unwrap_or_else(|| display.into());
        if let Ok(metadata) = fs::metadata(socket) {
            hash.update(metadata.dev().to_le_bytes());
            hash.update(metadata.ino().to_le_bytes());
        }
    } else if let Some(display) = env::var_os("DISPLAY") {
        hash.update([1]);
        hash.update(display.as_bytes());
    }
    // Existing launchers use this directory, so an in-flight game stays visible after migration.
    runtime
        .as_ref()
        .unwrap_or(&paths.data)
        .join("drabi-station")
        .join(format!("{:x}.lock", hash.finalize()))
}

pub fn start(paths: &Paths, id: &str) -> Result<i32> {
    paths.load_game(id)?;
    let cli = env::current_exe()?;
    let path = lease_path(paths);
    start_game(paths, &cli, &path, id)
}

pub fn status(paths: &Paths) -> Running {
    running(&lease_path(paths))
}

fn start_game(paths: &Paths, cli: &Path, path: &Path, id: &str) -> Result<i32> {
    fs::create_dir_all(path.parent().context("Station lock directory missing")?)?;
    let mut lease = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    match lease.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => match running(path) {
            Running::Game(id) => bail!("Game {id:?} is already running in this session"),
            _ => bail!("A managed game is already running in this session"),
        },
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    lease.set_len(0)?;
    lease.write_all(id.as_bytes())?;
    let mut command = Command::new(cli);
    command
        .arg("__session-run")
        .arg("--")
        .arg(id)
        .env_remove(crate::artwork::KEY_ENV);
    // The detached waiter owns the lock through UMU and post-exit cleanup.
    lifecycle::detach(paths, command, Some(lease))
}

pub fn run_game(paths: &Paths, id: &str) -> Result<i32> {
    handle_shutdown()?;
    let game = paths.load_game(id)?;
    let code = launch::run_registered(
        paths,
        game,
        launch::Options {
            managed: true,
            no_gamescope: true,
            ..Default::default()
        },
        || {
            let signal = STOP.load(Ordering::Relaxed);
            (signal != 0).then_some(128 + signal)
        },
    )?;
    if code != 0 {
        eprintln!("Managed game session {id:?} exited with status {code}");
    }
    Ok(code)
}

pub fn library(paths: &Paths) -> Result<Vec<GameCard>> {
    let mut cards = Vec::new();
    for id in crate::game_ids(paths)? {
        let file = match paths.load_game(&id) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("warning: skipping {id:?}: {error:#}");
                continue;
            }
        };
        let mut artwork = crate::artwork::local(paths, &id);
        let icon = artwork.remove("icon").or_else(|| {
            file.desktop
                .icon
                .as_deref()
                .and_then(|icon| paths.absolute(icon).ok())
                .filter(|path| path.is_file())
                .or_else(|| {
                    let path = paths.data.join("icons").join(format!("{id}.png"));
                    path.is_file().then_some(path)
                })
                .filter(|path| path.to_str().is_some())
        });
        let playtime = file
            .lifecycle
            .track_playtime
            .then(|| lifecycle::playtime(paths, &id).ok())
            .flatten();
        let last_played = lifecycle::last_played(paths, &id);
        let (publisher, version) = file
            .installation
            .map(|installation| (installation.publisher, installation.version))
            .unwrap_or_default();
        cards.push(GameCard {
            id,
            name: file.game.name,
            grid: artwork.remove("grid"),
            hero: artwork.remove("hero"),
            logo: artwork.remove("logo"),
            icon,
            publisher,
            version,
            last_played,
            playtime,
        });
    }
    cards.sort_by_cached_key(|card| card.name.to_lowercase());
    Ok(cards)
}

#[derive(Serialize)]
pub struct GameCard {
    pub id: String,
    pub name: String,
    pub grid: Option<PathBuf>,
    pub hero: Option<PathBuf>,
    pub logo: Option<PathBuf>,
    pub icon: Option<PathBuf>,
    pub publisher: Option<String>,
    pub version: Option<String>,
    pub last_played: Option<u64>,
    pub playtime: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Running {
    Idle,
    Game(String),
    Busy,
}

fn running(path: &std::path::Path) -> Running {
    use std::{fs::File, io::Read};
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Running::Idle,
        Err(_) => return Running::Busy,
    };
    match file.try_lock_shared() {
        Ok(()) => Running::Idle,
        Err(std::fs::TryLockError::WouldBlock) => {
            let mut id = String::new();
            if file.take(81).read_to_string(&mut id).is_ok()
                && !id.is_empty()
                && id.len() <= 80
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                Running::Game(id)
            } else {
                Running::Busy
            }
        }
        Err(_) => Running::Busy,
    }
}

use crate::{
    config::Paths,
    launch, lifecycle,
    station_protocol::{GameCard, Reply},
    station_system,
};
use anyhow::{Context, Result, ensure};
use std::{
    env, fs,
    io::Read,
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicI32, Ordering},
    thread,
    time::{Duration, Instant},
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

pub fn run(paths: &Paths, exit_after_game: bool, system: bool) -> Result<i32> {
    if system {
        station_system::require_unprivileged()?;
    }
    handle_shutdown()?;
    let cli = env::current_exe()?;
    let frontend = cli.with_file_name("drabi-station");

    while STOP.load(Ordering::Relaxed) == 0 {
        // The private directory holds one library snapshot and one response per frontend.
        let session = tempfile::Builder::new()
            .prefix("drabi-station-")
            .tempdir()?;
        let library_path = session.path().join("library.json");
        let reply_path = session.path().join("reply.json");
        let games = library(paths)?;
        serde_json::to_writer(fs::File::create(&library_path)?, &games)?;
        let mut command = Command::new(&frontend);
        command.arg(&library_path).arg(&reply_path).env(
            "DRABI_STATION_PREFERENCES",
            paths.config.join("station.json"),
        );
        if system {
            command.arg("--system");
        }
        let status = wait(&mut command).with_context(|| {
            format!(
                "Running {}. Install the optional Station frontend next to drabi",
                frontend.display()
            )
        })?;
        if STOP.load(Ordering::Relaxed) != 0 {
            break;
        }
        ensure!(status.success(), "Station frontend exited with {status}");
        let mut reply = Vec::new();
        fs::File::open(reply_path)
            .context("Station frontend did not send a response")?
            .take(1025)
            .read_to_end(&mut reply)?;
        ensure!(reply.len() <= 1024, "Station response exceeds 1024 bytes");
        let id = match serde_json::from_slice(&reply).context("Invalid Station response")? {
            Reply::Exit => return Ok(0),
            Reply::Select(id) => id,
            Reply::System(action) => {
                ensure!(system, "Station system actions require --system");
                drop(session);
                if STOP.load(Ordering::Relaxed) != 0 {
                    break;
                }
                station_system::perform(action)?;
                return Ok(0);
            }
        };
        ensure!(
            games.iter().any(|game| game.id == id),
            "Invalid Station game selection {id:?}"
        );
        drop(games);
        drop(session);

        if STOP.load(Ordering::Relaxed) != 0 {
            break;
        }
        let status = wait(
            Command::new(&cli)
                .env_remove(crate::artwork::KEY_ENV)
                .arg("__station-run")
                .arg("--")
                .arg(&id),
        );
        if STOP.load(Ordering::Relaxed) != 0 {
            break;
        }
        let code = match status {
            Ok(status) if status.success() => 0,
            Ok(status) => {
                eprintln!("error: game {id:?} exited with {status}");
                status
                    .code()
                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
            }
            Err(error) => {
                eprintln!("error: could not start game {id:?}: {error:#}");
                125
            }
        };
        if exit_after_game {
            return Ok(code);
        }
    }
    Ok(128 + STOP.load(Ordering::Relaxed))
}

pub fn run_game(paths: &Paths, id: &str) -> Result<i32> {
    handle_shutdown()?;
    let game = paths.load_game(id)?;
    launch::run_registered(
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
    )
}

fn wait(command: &mut Command) -> Result<ExitStatus> {
    let mut child = command.stdin(Stdio::null()).process_group(0).spawn()?;
    let mut deadline = None;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if STOP.load(Ordering::Relaxed) != 0 {
            match deadline {
                None => {
                    signal_group(&child, libc::SIGTERM);
                    deadline = Some(Instant::now() + Duration::from_secs(5));
                }
                Some(deadline) if Instant::now() >= deadline => {
                    signal_group(&child, libc::SIGKILL);
                    return Ok(child.wait()?);
                }
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn signal_group(child: &Child, signal: i32) {
    // The child leads its own process group, separate from Gamescope and Drabi.
    unsafe { libc::kill(-(child.id() as i32), signal) };
}

fn library(paths: &Paths) -> Result<Vec<GameCard>> {
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

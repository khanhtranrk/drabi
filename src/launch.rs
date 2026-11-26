use crate::{
    config::{GameFile, Paths, validate_env},
    lifecycle, performance, releases, runner, runtime,
};
use anyhow::{Context, Result, ensure};
use clap::Args;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

#[derive(Args, Default)]
pub struct Options {
    /// Use the specified Wine prefix directory.
    #[arg(long)]
    pub prefix: Option<PathBuf>,
    /// Select a Proton runner by name or path.
    #[arg(long)]
    pub runner: Option<String>,
    /// Override the UMU game ID used for Proton fixes.
    #[arg(long, value_name = "UMU_ID")]
    pub game_id: Option<String>,
    /// Override the UMU store identifier.
    #[arg(long)]
    pub store: Option<String>,
    /// Use a custom working directory.
    #[arg(long)]
    pub working_dir: Option<PathBuf>,
    /// Append a Windows argument. Use --arg=VALUE for arguments starting with a dash.
    #[arg(long = "arg", value_name = "VALUE")]
    pub arguments: Vec<String>,
    /// Override an environment variable as KEY=VALUE.
    #[arg(long = "env", value_name = "KEY=VALUE")]
    pub environment: Vec<String>,
    /// Enable Gamescope for this launch.
    #[arg(long, conflicts_with = "no_gamescope")]
    pub gamescope: bool,
    #[arg(long)]
    pub no_gamescope: bool,
    /// Enable GameMode for this launch.
    #[arg(long, conflicts_with = "no_gamemode")]
    pub gamemode: bool,
    #[arg(long)]
    pub no_gamemode: bool,
    /// Enable MangoHud for this launch.
    #[arg(long, conflicts_with = "no_mangohud")]
    pub mangohud: bool,
    #[arg(long)]
    pub no_mangohud: bool,
    /// Start in a separate session. Incompatible with lifecycle hooks and playtime.
    #[arg(long, conflicts_with = "managed")]
    pub detached: bool,
    /// Wait for the game and run configured lifecycle hooks, overriding saved detached mode.
    #[arg(long)]
    pub managed: bool,
    /// Print the launch plan without writing files or contacting the network.
    #[arg(long)]
    pub dry_run: bool,
    /// Show configured environment values in the dry-run output.
    #[arg(long, requires = "dry_run")]
    pub show_env: bool,
}

pub fn prefix_path(paths: &Paths, requested: Option<&Path>) -> Result<PathBuf> {
    match requested {
        Some(path) => paths.absolute(path),
        None => match paths.settings()?.default_prefix {
            Some(path) => {
                ensure!(
                    path.is_absolute() || path.starts_with("~"),
                    "default_prefix must be an absolute or ~/ path"
                );
                paths.absolute(&path)
            }
            None => Ok(paths.home.join(".wine")),
        },
    }
}

pub fn initialized(path: &Path) -> Result<bool> {
    let pfx = path.join("pfx");
    match fs::symlink_metadata(&pfx) {
        Ok(metadata) if metadata.is_symlink() => {
            let root = path.canonicalize()?;
            let destination = pfx
                .canonicalize()
                .with_context(|| format!("Cannot resolve prefix symlink {}", pfx.display()))?;
            ensure!(
                destination == root,
                "Prefix symlink {} points to {}. UMU would replace it. Select the actual prefix directly",
                pfx.display(),
                destination.display()
            );
        }
        Ok(metadata) if !metadata.is_dir() => {
            anyhow::bail!("Expected a directory at {}", pfx.display());
        }
        Ok(_) => return wine_directory_initialized(&pfx),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("Inspecting {}", pfx.display())),
    }
    wine_directory_initialized(path)
}

fn wine_directory_initialized(path: &Path) -> Result<bool> {
    let probe = |name| {
        let entry = path.join(name);
        match entry.metadata() {
            Ok(metadata) => Ok(Some(metadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("Inspecting {}", entry.display())),
        }
    };
    Ok(probe("drive_c")?.is_some_and(|entry| entry.is_dir())
        && probe("system.reg")?.is_some_and(|entry| entry.is_file()))
}

pub fn run(paths: &Paths, target: &str, options: Options) -> Result<i32> {
    let target_path = Path::new(target);
    let game = if !target.contains('/') && paths.game_path(target).is_ok_and(|path| path.exists()) {
        Some(paths.load_game(target)?)
    } else {
        None
    };
    let game_exe = game.as_ref().map(|file| file.game.exe.as_path());
    let exe = paths.absolute(game_exe.unwrap_or(target_path))?;
    ensure!(
        game.is_some()
            || exe
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe")),
        "Unknown game {target:?}. Use a registered ID or a path ending in .exe"
    );
    ensure!(exe.is_file(), "Executable not found at {}", exe.display());
    let directory = options
        .working_dir
        .as_deref()
        .or_else(|| {
            game.as_ref()
                .and_then(|file| file.game.working_dir.as_deref())
        })
        .or_else(|| exe.parent())
        .context("Cannot determine working directory")?;
    let directory = paths.absolute(directory)?;
    ensure!(
        directory.is_dir(),
        "Working directory not found at {}",
        directory.display()
    );

    let prefix = prefix_path(
        paths,
        options.prefix.as_deref().or_else(|| {
            game.as_ref()
                .and_then(|file| file.prefix.as_ref())
                .map(|prefix| prefix.path.as_path())
        }),
    )?;
    let proton = runner::resolve(
        paths,
        options.runner.as_deref().or_else(|| {
            game.as_ref()
                .and_then(|file| file.runner.as_ref())
                .map(|runner| runner.name.as_str())
        }),
    )?;

    let id = game.as_ref().map(|file| file.game.id.clone());
    let (mut performance, mut gamescope, lifecycle) =
        game.as_ref().map_or_else(Default::default, |file| {
            (
                file.performance.clone(),
                file.gamescope.clone(),
                file.lifecycle.clone(),
            )
        });
    performance.gamemode = !options.no_gamemode && (options.gamemode || performance.gamemode);
    performance.mangohud = !options.no_mangohud && (options.mangohud || performance.mangohud);
    gamescope.enabled = !options.no_gamescope && (options.gamescope || gamescope.enabled);
    let detached = !options.managed && (options.detached || lifecycle.detached);
    lifecycle.validate_mode(detached)?;
    let wrappers = performance::wrappers(&performance, &gamescope)?;

    let GameLaunch {
        game_id,
        store,
        mut environment,
        arguments,
    } = launch_settings(game, options.game_id, options.store, options.arguments);
    for item in options.environment {
        let (key, value) = item.split_once('=').context("--env expects KEY=VALUE")?;
        validate_env(key, value)?;
        environment.insert(key.to_owned(), value.to_owned());
    }

    add_compatibility_runtimes(paths, &mut environment);
    let mut argv = vec![exe.into_os_string()];
    argv.extend(arguments.into_iter().map(OsString::from));
    let active = runtime::entry(paths);
    let installed = runner::executable(&active);
    let umu = if options.dry_run {
        active.canonicalize().unwrap_or(active)
    } else {
        check_prefix(&prefix)?;
        let umu = runtime::ensure_installed(paths, false)?;
        check_prefix(&prefix)?;
        umu
    };
    let mut command = match wrappers.split_first() {
        Some((program, args)) => {
            let mut command = runtime_command(program, &prefix, &proton);
            command.args(args).arg(&umu);
            command
        }
        None => umu_command(&umu, &prefix, &proton),
    };
    command
        .current_dir(&directory)
        .args(&argv)
        .envs(&environment)
        .env("GAMEID", &game_id)
        .env("STORE", &store);
    if options.dry_run {
        let command_argv = std::iter::once(command.get_program())
            .chain(command.get_args())
            .collect::<Vec<_>>();
        println!("Command argv: {command_argv:?}");
        println!("Working directory: {}", directory.display());
        println!("Mode: {}", if detached { "detached" } else { "managed" });
        println!("WINEPREFIX={}", prefix.display());
        println!("PROTONPATH={}", proton.display());
        println!("GAMEID={game_id:?}");
        println!("STORE={store:?}");
        if options.show_env {
            for (key, value) in &environment {
                println!("{key}={value:?}");
            }
            let removed = command
                .get_envs()
                .filter_map(|(key, value)| value.is_none().then_some(key))
                .collect::<Vec<_>>();
            if !removed.is_empty() {
                println!("Removed inherited environment keys: {removed:?}");
            }
        } else if !environment.is_empty() {
            println!(
                "Additional environment keys: {:?}",
                environment.keys().collect::<Vec<_>>()
            );
        }
        if !installed {
            println!("UMU will be installed on the first real launch.");
        }
        check_prefix(&prefix)?;
        return Ok(0);
    }
    let runner_lease = releases::lease(paths, &proton)?;
    if detached {
        return lifecycle::detach(paths, command, runner_lease);
    }
    match lifecycle::run_hook("pre_launch", &lifecycle.pre_launch, &command, None) {
        Ok(Some(status)) if !status.success() => {
            eprintln!("error: pre_launch hook exited with {status}");
            let code = status_code(status);
            lifecycle::post_exit(&lifecycle, &command, code);
            return Ok(code);
        }
        Err(error) => {
            lifecycle::post_exit(&lifecycle, &command, 125);
            return Err(error);
        }
        _ => {}
    }

    let started = std::time::Instant::now();
    let code = match managed_status(&mut command) {
        Ok(status) => status_code(status),
        Err(error) => {
            if !lifecycle.pre_launch.is_empty() {
                lifecycle::post_exit(&lifecycle, &command, 125);
            }
            return Err(error);
        }
    };
    if lifecycle.track_playtime
        && let Some(id) = id.as_deref()
        && let Err(error) = lifecycle::record(paths, id, started.elapsed().as_secs())
    {
        eprintln!("warning: could not record playtime: {error:#}");
    }
    lifecycle::post_exit(&lifecycle, &command, code);
    Ok(code)
}

fn add_compatibility_runtimes(paths: &Paths, environment: &mut BTreeMap<String, String>) {
    let steam = paths.data.with_file_name("Steam").join("steamapps/common");
    let default_steam = paths.home.join(".local/share/Steam/steamapps/common");
    for (variable, directory) in [
        ("PROTON_EAC_RUNTIME", "Proton EasyAntiCheat Runtime"),
        ("PROTON_BATTLEYE_RUNTIME", "Proton BattlEye Runtime"),
    ] {
        if environment.contains_key(variable) || std::env::var_os(variable).is_some() {
            continue;
        }
        if let Some(path) = [steam.join(directory), default_steam.join(directory)]
            .into_iter()
            .find(|path| path.is_dir())
            && let Some(path) = path.to_str()
        {
            environment.insert(variable.to_owned(), path.to_owned());
        }
    }
}

struct GameLaunch {
    game_id: String,
    store: String,
    environment: BTreeMap<String, String>,
    arguments: Vec<String>,
}

fn launch_settings(
    game: Option<GameFile>,
    game_id: Option<String>,
    store: Option<String>,
    extra: Vec<String>,
) -> GameLaunch {
    let (umu, environment, mut arguments) = match game {
        Some(file) => (file.umu, file.environment, file.game.arguments),
        None => Default::default(),
    };
    arguments.extend(extra);
    GameLaunch {
        game_id: game_id
            .or(umu.game_id)
            .unwrap_or_else(|| "umu-default".into()),
        store: store.or(umu.store).unwrap_or_default(),
        environment,
        arguments,
    }
}

pub fn check_prefix(prefix: &Path) -> Result<()> {
    ensure!(
        initialized(prefix)?,
        "Prefix is missing or uninitialized at {}. Run drabi prefix create <path> with this path",
        prefix.display()
    );
    Ok(())
}

pub fn create_prefix(paths: &Paths, prefix: Option<&Path>, runner: Option<&str>) -> Result<i32> {
    let prefix = prefix_path(paths, prefix)?;
    check_creatable_prefix(&prefix)?;
    let proton = runner::resolve(paths, runner)?;
    let umu = runtime::ensure_installed(paths, false)?;
    fs::create_dir_all(&paths.data)?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.data.join(".prefix-create.lock"))?;
    lock.lock()?;
    check_creatable_prefix(&prefix)?;
    let _runner_lease = releases::lease(paths, &proton)?;
    let mut command = umu_command(&umu, &prefix, &proton);
    command.arg("");
    let code = exit_code(command)?;
    // UMU's empty target can initialize the prefix before Proton reports a missing executable.
    if initialized(&prefix)? {
        if let Err(error) = crate::prefixes::remember(paths, &prefix) {
            eprintln!("warning: could not record prefix location: {error:#}");
        }
        println!("Created prefix at {}", prefix.display());
        return Ok(0);
    }
    if code != 0 {
        anyhow::bail!(
            "UMU exited with code {code} without initializing {}. Inspect the partial directory or choose a new empty path",
            prefix.display()
        );
    }
    check_prefix(&prefix)?;
    Ok(0)
}

fn check_creatable_prefix(prefix: &Path) -> Result<()> {
    ensure!(
        !initialized(prefix)?,
        "Prefix already initialized at {}",
        prefix.display()
    );
    if prefix.exists() {
        ensure!(
            prefix.read_dir()?.next().is_none(),
            "Refusing to initialize a non-empty directory at {}. Inspect its contents or choose a new empty path",
            prefix.display()
        );
    }
    Ok(())
}

pub fn umu_command(umu: &Path, prefix: &Path, proton: &Path) -> Command {
    runtime_command(umu.as_os_str(), prefix, proton)
}

fn runtime_command(program: &OsStr, prefix: &Path, proton: &Path) -> Command {
    let mut command = Command::new(program);
    for (key, _) in std::env::vars_os() {
        if key.as_encoded_bytes().starts_with(b"UMU_")
            || matches!(
                key.to_str(),
                Some("PROTON_VERB" | "STEAM_COMPAT_DATA_PATH" | "STEAM_COMPAT_INSTALL_PATH")
            )
        {
            command.env_remove(key);
        }
    }
    command
        .env("WINEPREFIX", prefix)
        .env("PROTONPATH", proton)
        .env("GAMEID", "umu-default")
        .env("STORE", "");
    command
}

pub fn exit_code(mut command: Command) -> Result<i32> {
    managed_status(&mut command).map(status_code)
}

fn managed_status(command: &mut Command) -> Result<ExitStatus> {
    command
        .status()
        .context("Starting the managed UMU launcher")
}

fn status_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

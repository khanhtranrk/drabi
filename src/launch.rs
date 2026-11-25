use crate::{
    config::{GameFile, Paths, validate_env},
    runner, runtime,
};
use anyhow::{Context, Result, ensure};
use clap::Args;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
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
    /// Print the launch plan without writing files or contacting the network.
    #[arg(long)]
    pub dry_run: bool,
}

pub fn prefix_path(paths: &Paths, requested: Option<&Path>) -> Result<PathBuf> {
    match requested {
        Some(path) => paths.absolute(path),
        None => Ok(paths.home.join(".wine")),
    }
}

pub fn initialized(path: &Path) -> bool {
    let initialized_at =
        |dir: &Path| dir.join("drive_c").is_dir() && dir.join("system.reg").is_file();
    initialized_at(path) || initialized_at(&path.join("pfx"))
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

    let mut argv = vec![exe.into_os_string()];
    argv.extend(arguments.into_iter().map(OsString::from));
    if options.dry_run {
        println!("UMU: {}", runtime::entry(paths).display());
        println!("Working directory: {}", directory.display());
        println!("Arguments: {argv:?}");
        println!("WINEPREFIX={}", prefix.display());
        println!("PROTONPATH={}", proton.display());
        println!("GAMEID={game_id:?}");
        println!("STORE={store:?}");
        if !environment.is_empty() {
            println!(
                "Additional environment keys: {:?}",
                environment.keys().collect::<Vec<_>>()
            );
        }
        if !runner::executable(&runtime::entry(paths)) {
            println!("UMU will be installed on the first real launch.");
        }
        check_prefix(&prefix)?;
        return Ok(0);
    }
    check_prefix(&prefix)?;
    let umu = runtime::ensure_installed(paths, false)?;
    check_prefix(&prefix)?;
    let mut command = umu_command(&umu, &prefix, &proton);
    command
        .current_dir(directory)
        .args(argv)
        .envs(environment)
        .env("GAMEID", game_id)
        .env("STORE", store);
    exit_code(command)
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
        initialized(prefix),
        "Prefix is missing or uninitialized at {}. Run drabi prefix create <path> with this path",
        prefix.display()
    );
    Ok(())
}

pub fn create_prefix(paths: &Paths, prefix: &Path, runner: Option<&str>) -> Result<i32> {
    let prefix = prefix_path(paths, Some(prefix))?;
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
    let mut command = umu_command(&umu, &prefix, &proton);
    command.arg("");
    let code = exit_code(command)?;
    // UMU's empty target can initialize the prefix before Proton reports a missing executable.
    if initialized(&prefix) {
        println!("Created prefix at {}", prefix.display());
        return Ok(0);
    }
    if code != 0 {
        anyhow::bail!(
            "UMU exited with code {code} without initializing {}",
            prefix.display()
        );
    }
    check_prefix(&prefix)?;
    Ok(0)
}

fn check_creatable_prefix(prefix: &Path) -> Result<()> {
    ensure!(
        !initialized(prefix),
        "Prefix already initialized at {}",
        prefix.display()
    );
    if prefix.exists() {
        ensure!(
            prefix.read_dir()?.next().is_none(),
            "Refusing to initialize a non-empty directory at {}",
            prefix.display()
        );
    }
    Ok(())
}

fn umu_command(umu: &Path, prefix: &Path, proton: &Path) -> Command {
    let mut command = Command::new(umu);
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

fn exit_code(mut command: Command) -> Result<i32> {
    use std::os::unix::process::ExitStatusExt;
    let status = command
        .status()
        .context("Starting the managed UMU launcher")?;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}

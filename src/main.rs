mod artwork;
mod config;
mod desktop;
mod installation;
mod launch;
mod lifecycle;
mod output;
mod performance;
mod prefixes;
mod releases;
mod runner;
mod runtime;
mod station;
mod station_protocol;
mod station_system;

use anyhow::{Context, Result, ensure};
use clap::{CommandFactory, Parser, Subcommand};
use config::{Game, GameFile, Paths, Prefix, Runner, atomic_write, parse_game};
use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::PathBuf,
    process::{Command, ExitCode},
};

#[derive(Parser)]
#[command(version, about = "Launch Windows games on Linux with UMU and Proton")]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Run a registered game or a Windows executable.
    Run {
        /// Saved game ID or path to an .exe file.
        target: String,
        #[command(flatten)]
        options: launch::Options,
    },
    /// Open the fullscreen game library.
    Station {
        /// Exit with the game result after one launch attempt.
        #[arg(long)]
        exit_after_game: bool,
        /// Enable the Logout, Restart and Shutdown menu.
        #[arg(long)]
        system: bool,
    },
    #[command(name = "__station-run", hide = true)]
    StationRun { id: String },
    /// Install, uninstall, register and manage games and shortcuts.
    Game {
        #[command(subcommand)]
        command: GameAction,
    },
    /// Discover and register Proton runners.
    Proton {
        #[command(subcommand)]
        command: ProtonAction,
    },
    /// Inspect or initialize Wine prefixes.
    Prefix {
        #[command(subcommand)]
        command: PrefixAction,
    },
    /// Check local runtime, runners and game configurations.
    Doctor,
    /// Check for a new UMU release immediately.
    Update,
    #[command(name = "__wait-runner", hide = true)]
    WaitRunner {
        fd: i32,
        #[arg(required = true, last = true)]
        argv: Vec<std::ffi::OsString>,
    },
    /// Generate a completion script for a shell.
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Print the drabi version.
    Version,
}

#[derive(Subcommand)]
enum GameAction {
    /// Register a game. Omitted required fields are prompted for in a terminal.
    Add {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        exe: Option<PathBuf>,
        #[arg(long)]
        runner: Option<String>,
        #[arg(long)]
        prefix: Option<PathBuf>,
        #[arg(long)]
        working_dir: Option<PathBuf>,
    },
    /// Run an installer and register the detected game.
    Install(installation::InstallOptions),
    /// Run the registered Windows uninstaller for a game.
    Uninstall {
        id: String,
        /// Delete the game's dedicated Drabi-owned prefix after uninstalling.
        #[arg(long)]
        purge: bool,
    },
    /// Create or update an application-menu shortcut.
    CreateShortcut {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        icon: Option<PathBuf>,
        /// Preview the shortcut and icon without writing files.
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove an application-menu shortcut.
    RemoveShortcut { id: String },
    /// List registrations.
    List {
        #[arg(long, value_enum, default_value_t = output::Format::Text)]
        format: output::Format,
    },
    /// Print a game's TOML configuration.
    Show {
        id: String,
        #[arg(long, value_enum, default_value_t = output::Format::Text)]
        format: output::Format,
    },
    /// Edit a game using VISUAL or EDITOR.
    Edit { id: String },
    /// Fetch or refresh locally saved SteamGridDB artwork.
    Artwork {
        id: String,
        /// Select a specific SteamGridDB game ID.
        #[arg(long)]
        provider_id: Option<u64>,
    },
    /// Show recorded playtime in seconds.
    Playtime {
        id: String,
        #[arg(long, value_enum, default_value_t = output::Format::Text)]
        format: output::Format,
    },
    /// Remove a game's registration.
    Remove { id: String },
}

#[derive(Subcommand)]
enum ProtonAction {
    /// List local runners or browse installable versions.
    List {
        /// Query the available release versions.
        #[arg(long)]
        available: bool,
        #[arg(long, value_enum, default_value_t = output::Format::Text)]
        format: output::Format,
        #[arg(long, value_enum, requires = "available")]
        source: Option<releases::Source>,
        #[arg(long, requires = "available", value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: Option<u32>,
    },
    /// Discover runners in the drabi Proton directory.
    Scan {
        #[arg(long, value_enum, default_value_t = output::Format::Text)]
        format: output::Format,
    },
    /// Search versions from a Proton source.
    Search {
        query: Option<String>,
        #[arg(long, value_enum, default_value_t = output::Format::Text)]
        format: output::Format,
        #[arg(long, value_enum, default_value = "ge")]
        source: releases::Source,
        #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Install an exact version or the latest from a Proton family.
    Install { name: String },
    /// Install the latest release alongside existing versions.
    Update { name: String },
    /// Remove a drabi-managed Proton version.
    Remove { name: String },
    /// Register an existing runner directory.
    Register { name: String, path: PathBuf },
    /// Remove an external runner registration.
    Unregister { name: String },
}

#[derive(Subcommand)]
enum PrefixAction {
    /// List the default, registered and previously created prefixes.
    List {
        #[arg(long, value_enum, default_value_t = output::Format::Text)]
        format: output::Format,
    },
    /// Open Wine configuration for an initialized prefix.
    Winecfg {
        path: Option<PathBuf>,
        #[arg(long)]
        runner: Option<String>,
    },
    /// Open Registry Editor for an initialized prefix.
    Regedit {
        path: Option<PathBuf>,
        #[arg(long)]
        runner: Option<String>,
    },
    /// Open the Windows Control Panel for an initialized prefix.
    Control {
        path: Option<PathBuf>,
        #[arg(long)]
        runner: Option<String>,
    },
    /// Run one or more Winetricks verbs in an initialized prefix.
    Winetricks {
        /// Winetricks verbs. An explicit path as the first argument is also accepted.
        #[arg(required = true)]
        verbs: Vec<String>,
        /// Use this prefix instead of the configured default.
        #[arg(long)]
        prefix: Option<PathBuf>,
        #[arg(long)]
        runner: Option<String>,
    },
    /// Show whether a prefix is initialized.
    Inspect {
        path: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = output::Format::Text)]
        format: output::Format,
    },
    /// Initialize a prefix with the chosen runner.
    Create {
        path: Option<PathBuf>,
        #[arg(long)]
        runner: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match execute(cli.command) {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn execute(action: Action) -> Result<i32> {
    if matches!(action, Action::Version) {
        println!("drabi {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if let Action::WaitRunner { fd, argv } = action {
        return lifecycle::wait_runner(fd, &argv);
    }
    if let Action::Completions { shell } = action {
        clap_complete::generate(shell, &mut Cli::command(), "drabi", &mut io::stdout());
        return Ok(0);
    }
    let paths = Paths::load()?;
    match action {
        Action::Station {
            exit_after_game,
            system,
        } => {
            return station::run(&paths, exit_after_game, system);
        }
        Action::StationRun { id } => return station::run_game(&paths, &id),
        Action::Run { target, options } => return launch::run(&paths, &target, options),
        Action::Game { command } => game_action(&paths, command)?,
        Action::Proton { command } => match command {
            ProtonAction::Search {
                query,
                limit,
                source,
                format,
            } => {
                let rows = releases::search(&paths, source, query.as_deref(), limit as usize)?;
                releases::print_releases(format, &rows)?;
            }
            ProtonAction::Install { name } => releases::install(&paths, &name)?,
            ProtonAction::Update { name } => releases::update(&paths, &name)?,
            ProtonAction::Remove { name } => releases::remove(&paths, &name)?,
            ProtonAction::Register { name, path } => {
                runner::update_registration(&paths, &name, Some(&path))?;
                println!("Registered {name}");
            }
            ProtonAction::Unregister { name } => {
                runner::update_registration(&paths, &name, None)?;
                println!("Unregistered {name}");
            }
            ProtonAction::List {
                available: true,
                source,
                limit,
                format,
            } => {
                let rows = releases::available(&paths, source, limit.unwrap_or(5) as usize)?;
                releases::print_releases(format, &rows)?;
            }
            ProtonAction::List {
                available: false,
                format,
                ..
            } => list_runners(&paths, true, format)?,
            ProtonAction::Scan { format } => list_runners(&paths, false, format)?,
        },
        Action::Prefix { command } => match command {
            PrefixAction::List { format } => prefixes::list(&paths, format)?,
            PrefixAction::Winecfg { path, runner } => {
                return prefixes::tool(
                    &paths,
                    path.as_deref(),
                    runner.as_deref(),
                    "winecfg.exe",
                    &[],
                );
            }
            PrefixAction::Regedit { path, runner } => {
                return prefixes::tool(
                    &paths,
                    path.as_deref(),
                    runner.as_deref(),
                    "regedit.exe",
                    &[],
                );
            }
            PrefixAction::Control { path, runner } => {
                return prefixes::tool(
                    &paths,
                    path.as_deref(),
                    runner.as_deref(),
                    "control.exe",
                    &[],
                );
            }
            PrefixAction::Winetricks {
                prefix,
                runner,
                verbs,
            } => return prefixes::winetricks(&paths, prefix.as_deref(), runner.as_deref(), &verbs),
            PrefixAction::Create { path, runner } => {
                return launch::create_prefix(&paths, path.as_deref(), runner.as_deref());
            }
            PrefixAction::Inspect { path, format } => {
                prefixes::inspect(&paths, path.as_deref(), format)?
            }
        },
        Action::Doctor => return doctor(&paths),
        Action::Update => println!("{}", runtime::update(&paths)?.display()),
        Action::Version | Action::WaitRunner { .. } | Action::Completions { .. } => unreachable!(),
    }
    Ok(0)
}

#[derive(serde::Serialize)]
struct RunnerRow {
    name: String,
    source: &'static str,
    path: PathBuf,
}

fn list_runners(paths: &Paths, external_requested: bool, format: output::Format) -> Result<()> {
    let managed = runner::scan(paths)?;
    let external = if external_requested {
        paths.settings()?.external_runners
    } else {
        Default::default()
    };
    let mut rows = Vec::new();
    if external_requested {
        rows.extend(
            releases::steam_runners(paths)
                .filter(|(name, _)| !managed.contains_key(*name) && !external.contains_key(*name))
                .map(|(name, path)| RunnerRow {
                    name: name.into(),
                    source: "steam",
                    path,
                }),
        );
    }
    rows.extend(managed.into_iter().map(|(name, path)| {
        let source = if releases::is_managed(&path) {
            "managed"
        } else {
            "discovered"
        };
        RunnerRow { name, source, path }
    }));
    rows.extend(external.into_iter().map(|(name, path)| RunnerRow {
        name,
        source: "external",
        path,
    }));
    output::list(
        format,
        ["Runner", "Source", "Path"],
        &rows,
        |row| {
            [
                row.name.clone(),
                row.source.into(),
                row.path.display().to_string(),
            ]
        },
        "No Proton runners found.",
    )
}

#[derive(serde::Serialize)]
struct GameRow {
    id: String,
    name: String,
}

fn game_ids(paths: &Paths) -> Result<Vec<String>> {
    let files = match fs::read_dir(paths.config.join("games")) {
        Ok(files) => files,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut ids = Vec::new();
    for file in files {
        let path = file?.path();
        if !path
            .file_name()
            .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."))
            && path.extension().is_some_and(|ext| ext == "toml")
            && let Some(id) = path.file_stem().and_then(|id| id.to_str())
        {
            ids.push(id.to_owned());
        }
    }
    ids.sort();
    Ok(ids)
}

fn game_action(paths: &Paths, action: GameAction) -> Result<()> {
    match action {
        GameAction::Add {
            id,
            name,
            exe,
            runner,
            prefix,
            working_dir,
        } => {
            let id = required(id, "--id", "Game ID")?;
            config::validate_id(&id)?;
            let name = required(name, "--name", "Display name")?;
            let exe = match exe {
                Some(path) => path,
                None => PathBuf::from(required(None, "--exe", "Executable path")?),
            };
            let exe = paths.absolute(&exe)?;
            ensure!(exe.is_file(), "Executable not found at {}", exe.display());

            let file = GameFile::new(
                Game {
                    id,
                    name,
                    exe,
                    working_dir: working_dir
                        .as_deref()
                        .map(|path| paths.absolute(path))
                        .transpose()?,
                    arguments: Vec::new(),
                },
                runner.map(|name| Runner { name }),
                prefix
                    .as_deref()
                    .map(|path| paths.absolute(path).map(|path| Prefix { path }))
                    .transpose()?,
            );
            file.register(paths)?;
            artwork::discover(paths, &file);
        }
        GameAction::Install(options) => installation::install(paths, options)?,
        GameAction::Uninstall { id, purge } => installation::uninstall(paths, &id, purge)?,
        GameAction::CreateShortcut {
            id,
            name,
            icon,
            dry_run,
        } => {
            desktop::create(paths, &id, name.as_deref(), icon.as_deref(), dry_run)?;
        }
        GameAction::RemoveShortcut { id } => desktop::remove(paths, &id)?,
        GameAction::List { format } => {
            let mut rows = Vec::new();
            let mut invalid = false;
            for id in game_ids(paths)? {
                match paths.load_game(&id) {
                    Ok(file) => rows.push(GameRow {
                        id,
                        name: file.game.name,
                    }),
                    Err(error) => {
                        eprintln!("error: {id}: {error:#}");
                        invalid = true;
                    }
                }
            }
            output::list(
                format,
                ["ID", "Name"],
                &rows,
                |row| [row.id.clone(), row.name.clone()],
                "No games registered.",
            )?;
            ensure!(
                !invalid,
                "One or more game configurations could not be read"
            );
        }
        GameAction::Show { id, format } => {
            if matches!(format, output::Format::Json) {
                output::json(&paths.load_game(&id)?)?;
            } else {
                let path = paths.game_path(&id)?;
                print!(
                    "{}",
                    fs::read_to_string(&path)
                        .with_context(|| format!("Reading {}", path.display()))?
                );
            }
        }
        GameAction::Edit { id } => edit_game(paths, &id)?,
        GameAction::Artwork { id, provider_id } => {
            artwork::fetch(paths, &paths.load_game(&id)?, provider_id, true)?;
        }
        GameAction::Playtime { id, format } => {
            paths.load_game(&id)?;
            let seconds = lifecycle::playtime(paths, &id)?;
            if matches!(format, output::Format::Json) {
                output::json(&serde_json::json!({ "id": id, "seconds": seconds }))?;
            } else {
                println!("{seconds} seconds");
            }
        }
        GameAction::Remove { id } => {
            let path = paths.game_path(&id)?;
            let _lock = paths.lock_config()?;
            fs::remove_file(&path).with_context(|| format!("Removing {}", path.display()))?;
            artwork::remove(paths, &id);
            lifecycle::forget_recent(paths, &id);
            println!("Removed registration {id}");
            if desktop::entry_path(paths, &id)?.is_file() {
                eprintln!(
                    "warning: shortcut remains. Remove it with drabi game remove-shortcut {id}"
                );
            }
        }
    }
    Ok(())
}

fn required(value: Option<String>, flag: &str, label: &str) -> Result<String> {
    if let Some(value) = value {
        return Ok(value);
    }
    ensure!(
        io::stdin().is_terminal(),
        "Missing {flag} for non-interactive registration"
    );
    eprint!("{label}: ");
    io::stderr().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    ensure!(!value.trim().is_empty(), "{label} is required");
    Ok(value.trim().to_owned())
}

fn edit_game(paths: &Paths, id: &str) -> Result<()> {
    let path = paths.game_path(id)?;
    let original =
        fs::read_to_string(&path).with_context(|| format!("Reading {}", path.display()))?;
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| std::env::var("EDITOR").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "vi".into());
    let words = shell_words::split(&editor).context("Parsing the editor setting")?;
    let (program, arguments) = words.split_first().context("Editor command is empty")?;
    let directory = path.parent().context("Game path has no parent")?;
    // The staged filename retains its extension for editor syntax detection.
    let mut staged = tempfile::Builder::new()
        .prefix(".edit-")
        .suffix(".toml")
        .tempfile_in(directory)?;
    staged.write_all(original.as_bytes())?;
    let staged_path = staged.into_temp_path();
    let status = Command::new(program)
        .args(arguments)
        .arg(&staged_path)
        .status()
        .context("Starting the editor")?;
    ensure!(status.success(), "Editor exited with {status}");
    let edited = fs::read_to_string(&staged_path)?;
    let _lock = paths.lock_config()?;
    let validation = parse_game(&edited, id).and_then(|_| {
        ensure!(
            fs::read_to_string(&path)? == original,
            "The game changed while the editor was open"
        );
        Ok(())
    });
    if let Err(error) = validation {
        let recovery = staged_path.keep()?;
        return Err(error.context(format!(
            "Edited copy is available at {}",
            recovery.display()
        )));
    }
    if edited != original {
        atomic_write(&path, &edited, true)?;
    }
    Ok(())
}

fn doctor(paths: &Paths) -> Result<i32> {
    let umu = runtime::entry(paths);
    let installed = runner::executable(&umu);
    println!(
        "UMU: {}",
        if installed {
            umu.display().to_string()
        } else {
            "missing. Use drabi update to install".into()
        }
    );
    let mut failed = !installed;
    for dependency in ["curl", "python3"] {
        let found = std::env::var_os("PATH").unwrap_or_default();
        let available =
            std::env::split_paths(&found).any(|dir| runner::executable(&dir.join(dependency)));
        println!(
            "{dependency}: {}",
            if available {
                "found"
            } else {
                "missing from PATH"
            }
        );
        failed |= !available;
    }
    for tool in ["gamescope", "gamemoderun", "mangohud", "mangoapp"] {
        let state = if performance::find_tool(tool).is_some() {
            "found"
        } else {
            "missing (optional)"
        };
        println!("{tool}: {state}");
    }
    let settings = paths.settings()?;
    for (name, path) in &settings.external_runners {
        if !runner::executable(&paths.absolute(path)?.join("proton")) {
            eprintln!(
                "error: runner {name:?} has no executable proton script at {}",
                path.display()
            );
            failed = true;
        }
    }
    let managed = runner::scan(paths)?;
    let steam_count = releases::steam_runners(paths).count();
    println!(
        "Proton: {} discovered, {} registered, {steam_count} Steam",
        managed.len(),
        settings.external_runners.len()
    );
    if managed.is_empty() && settings.external_runners.is_empty() && steam_count == 0 {
        eprintln!("error: use drabi proton register <name> <path> to register Proton");
        failed = true;
    }
    for id in game_ids(paths)? {
        let result = paths.load_game(&id).and_then(|file| {
            let exe = paths.absolute(&file.game.exe)?;
            ensure!(exe.is_file(), "Executable not found at {}", exe.display());
            if let Some(directory) = file.game.working_dir {
                let directory = paths.absolute(&directory)?;
                ensure!(
                    directory.is_dir(),
                    "Working directory not found at {}",
                    directory.display()
                );
            }
            runner::resolve(
                paths,
                file.runner.as_ref().map(|runner| runner.name.as_str()),
            )?;
            let prefix = launch::prefix_path(
                paths,
                file.prefix.as_ref().map(|prefix| prefix.path.as_path()),
            )?;
            launch::check_prefix(&prefix)?;
            performance::wrappers(&file.performance, &file.gamescope)?;
            file.lifecycle.validate_mode(file.lifecycle.detached)?;
            Ok(())
        });
        match result {
            Ok(()) => println!("{id}: ready"),
            Err(error) => {
                failed = true;
                eprintln!("error: {id}: {error:#}");
            }
        }
    }
    Ok(i32::from(failed))
}

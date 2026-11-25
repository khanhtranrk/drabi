mod config;
mod launch;
mod runner;
mod runtime;

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
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
    /// Manage saved game configurations.
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
    /// List registrations.
    List,
    /// Print a game's TOML configuration.
    Show { id: String },
    /// Edit a game using VISUAL or EDITOR.
    Edit { id: String },
    /// Remove a game's registration.
    Remove { id: String },
}

#[derive(Subcommand)]
enum ProtonAction {
    /// List discovered and registered runners.
    List,
    /// Discover runners in the drabi Proton directory.
    Scan,
    /// Register an existing runner directory.
    Register { name: String, path: PathBuf },
    /// Remove an external runner registration.
    Unregister { name: String },
}

#[derive(Subcommand)]
enum PrefixAction {
    /// Show whether a prefix is initialized.
    Inspect { path: PathBuf },
    /// Initialize a prefix with the chosen runner.
    Create {
        path: PathBuf,
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
    let paths = Paths::load()?;
    match action {
        Action::Run { target, options } => return launch::run(&paths, &target, options),
        Action::Game { command } => game_action(&paths, command)?,
        Action::Proton { command } => match command {
            ProtonAction::Register { name, path } => {
                runner::update_registration(&paths, &name, Some(&path))?;
                println!("Registered {name}");
            }
            ProtonAction::Unregister { name } => {
                runner::update_registration(&paths, &name, None)?;
                println!("Unregistered {name}");
            }
            ProtonAction::List => list_runners(&paths, true)?,
            ProtonAction::Scan => list_runners(&paths, false)?,
        },
        Action::Prefix { command } => match command {
            PrefixAction::Create { path, runner } => {
                return launch::create_prefix(&paths, &path, runner.as_deref());
            }
            PrefixAction::Inspect { path } => {
                let path = launch::prefix_path(&paths, Some(&path))?;
                launch::check_prefix(&path)?;
                println!("Initialized prefix: {}", path.display());
            }
        },
        Action::Doctor => return doctor(&paths),
        Action::Update => println!("{}", runtime::ensure_installed(&paths, true)?.display()),
        Action::Version => unreachable!(),
    }
    Ok(0)
}

fn list_runners(paths: &Paths, external: bool) -> Result<()> {
    let managed = runner::scan(paths)?;
    let external = if external {
        paths.settings()?.external_runners
    } else {
        Default::default()
    };
    if managed.is_empty() && external.is_empty() {
        println!("No Proton runners found.");
    }
    for (name, path) in managed {
        println!("{name}\tdiscovered\t{}", path.display());
    }
    for (name, path) in external {
        println!("{name}\texternal\t{}", path.display());
    }
    Ok(())
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
            let path = paths.game_path(&id)?;
            let name = required(name, "--name", "Display name")?;
            let exe = match exe {
                Some(path) => path,
                None => PathBuf::from(required(None, "--exe", "Executable path")?),
            };
            let exe = paths.absolute(&exe)?;
            ensure!(exe.is_file(), "Executable not found at {}", exe.display());

            let file = GameFile {
                game: Game {
                    id,
                    name,
                    exe,
                    working_dir: working_dir
                        .as_deref()
                        .map(|path| paths.absolute(path))
                        .transpose()?,
                    arguments: Vec::new(),
                },
                runner: runner.map(|name| Runner { name }),
                prefix: prefix
                    .as_deref()
                    .map(|path| paths.absolute(path).map(|path| Prefix { path }))
                    .transpose()?,
                umu: Default::default(),
                environment: Default::default(),
            };
            let raw = toml::to_string_pretty(&file)?;
            parse_game(&raw, &file.game.id)?;
            let _lock = paths.lock_config()?;
            atomic_write(&path, &raw, false)
                .with_context(|| format!("Registering {}", file.game.id))?;
            println!("Registered {}", file.game.id);
        }
        GameAction::List => {
            let ids = game_ids(paths)?;
            if ids.is_empty() {
                println!("No games registered.");
            }
            let mut invalid = false;
            for id in ids {
                match paths.load_game(&id) {
                    Ok(file) => println!("{id}\t{}", file.game.name),
                    Err(error) => {
                        eprintln!("error: {id}: {error:#}");
                        invalid = true;
                    }
                }
            }
            ensure!(
                !invalid,
                "One or more game configurations could not be read"
            );
        }
        GameAction::Show { id } => {
            let path = paths.game_path(&id)?;
            print!(
                "{}",
                fs::read_to_string(&path).with_context(|| format!("Reading {}", path.display()))?
            );
        }
        GameAction::Edit { id } => edit_game(paths, &id)?,
        GameAction::Remove { id } => {
            let path = paths.game_path(&id)?;
            let _lock = paths.lock_config()?;
            fs::remove_file(&path).with_context(|| format!("Removing {}", path.display()))?;
            println!("Removed registration {id}");
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
    println!(
        "Proton: {} discovered, {} registered",
        managed.len(),
        settings.external_runners.len()
    );
    if managed.is_empty() && settings.external_runners.is_empty() {
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
            launch::check_prefix(&prefix)
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

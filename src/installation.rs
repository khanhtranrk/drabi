mod discovery;

use crate::{
    config::{Game, GameFile, Installation, Paths, Prefix, Runner, atomic_write, validate_id},
    desktop, launch, runner,
};
use anyhow::{Context, Result, bail, ensure};
use discovery::{
    Candidate, Snapshot, drive_root, find_uninstall, is_exe, registry, windows_path, wine_root,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

#[derive(clap::Args)]
pub struct InstallOptions {
    /// Windows installer to run.
    installer: PathBuf,
    /// Register under this ID instead of deriving it from the detected name.
    #[arg(long)]
    id: Option<String>,
    /// Use this prefix instead of the default.
    #[arg(long, conflicts_with = "isolated_prefix")]
    prefix: Option<PathBuf>,
    /// Create a dedicated Drabi-owned prefix for this game.
    #[arg(long)]
    isolated_prefix: bool,
    /// Use this Proton runner instead of the default.
    #[arg(long)]
    runner: Option<String>,
    /// Pass an argument to the installer. Use --arg=VALUE for an argument starting with a dash.
    #[arg(long = "arg")]
    arguments: Vec<String>,
}

const OWNER: &str = ".drabi-game-owner.json";

#[derive(Serialize, Deserialize)]
struct Owner {
    game_id: String,
    initial_registry_keys: BTreeSet<String>,
}

fn operation_lock(paths: &Paths) -> Result<File> {
    fs::create_dir_all(&paths.data)?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.data.join(".game-installation.lock"))?;
    lock.try_lock()
        .context("Cannot start a second game installation or uninstallation")?;
    Ok(lock)
}

fn isolated_root(paths: &Paths) -> Result<PathBuf> {
    let root = paths.data.join("game-prefixes");
    ensure!(
        !root.is_symlink(),
        "Refusing a symlink at {}",
        root.display()
    );
    Ok(root)
}

pub fn install(paths: &Paths, options: InstallOptions) -> Result<()> {
    let installer = paths.absolute(&options.installer)?;
    ensure!(
        is_exe(&installer) && installer.is_file(),
        "Expected an existing .exe installer at {}",
        installer.display()
    );
    let installer = installer.canonicalize()?;
    if let Some(id) = &options.id {
        validate_id(id)?;
        ensure!(
            !paths.game_path(id)?.exists(),
            "Game {id} is already registered"
        );
    }

    let _operation = operation_lock(paths)?;
    let proton = runner::resolve(paths, options.runner.as_deref())?;
    let prefix = if options.isolated_prefix {
        let root = isolated_root(paths)?;
        fs::create_dir_all(&root)?;
        tempfile::Builder::new()
            .prefix("game-")
            .tempdir_in(root)?
            .keep()
    } else {
        launch::prefix_path(paths, options.prefix.as_deref())?
    };
    let result = finish_install(paths, &installer, &prefix, proton, options);
    result.with_context(|| {
        format!(
            "Game installation in {}. Installed files and prefix have been kept",
            prefix.display()
        )
    })
}

fn finish_install(
    paths: &Paths,
    installer: &Path,
    prefix: &Path,
    proton: PathBuf,
    options: InstallOptions,
) -> Result<()> {
    let pinned_runner = proton
        .to_str()
        .context("Proton path is not UTF-8")?
        .to_owned();
    if !launch::initialized(prefix)? {
        launch::create_prefix(paths, Some(prefix), Some(&pinned_runner))?;
    }
    let prefix = prefix.canonicalize()?;
    let root = wine_root(&prefix);
    let before = Snapshot::read(&root)?;
    println!("Running installer in {}", prefix.display());
    let status = launch::run(
        paths,
        installer.to_str().context("Installer path is not UTF-8")?,
        launch::Options {
            prefix: Some(prefix.clone()),
            runner: Some(pinned_runner.clone()),
            arguments: options.arguments,
            ..Default::default()
        },
    )?;
    ensure!(status == 0, "Installer exited with status {status}");

    let after = Snapshot::read(&root)?;
    let candidate = select(discovery::candidates(&root, &before, &after, installer))?;
    let id = match options.id {
        Some(id) => id,
        None => available_id(paths, &candidate.name)?,
    };
    let mut file = GameFile::new(
        Game {
            id: id.clone(),
            name: candidate.name,
            working_dir: candidate
                .directory
                .or_else(|| candidate.path.parent().map(Path::to_owned)),
            exe: candidate.path,
            arguments: candidate.arguments,
        },
        Some(Runner {
            name: pinned_runner,
        }),
        Some(Prefix {
            path: prefix.clone(),
        }),
    );
    file.installation = Some(Installation {
        registry_key: candidate.registry_key,
        publisher: candidate.publisher,
        version: candidate.version,
    });
    if options.isolated_prefix {
        let owner = Owner {
            game_id: id,
            initial_registry_keys: before.registry.into_keys().collect(),
        };
        atomic_write(&prefix.join(OWNER), &serde_json::to_string(&owner)?, false)?;
    }
    file.register(paths)?;
    crate::artwork::discover(paths, &file);
    Ok(())
}

fn available_id(paths: &Paths, name: &str) -> Result<String> {
    let slug = name
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase();
    let base = if slug.is_empty() {
        "game"
    } else {
        &slug[..slug.len().min(64)]
    };
    for suffix in 1..=10_000 {
        let id = if suffix == 1 {
            base.to_owned()
        } else {
            format!("{base}-{suffix}")
        };
        if !paths.game_path(&id)?.exists() {
            return Ok(id);
        }
    }
    bail!("Could not derive an unused game ID. Pass --id")
}

fn select(mut candidates: Vec<Candidate>) -> Result<Candidate> {
    if candidates.len() == 1
        || (candidates.len() > 1
            && candidates[0].score >= 25
            && candidates[0].score - candidates[1].score >= 20)
    {
        return Ok(candidates.remove(0));
    }
    ensure!(
        io::stdin().is_terminal(),
        "No unique game executable was found. Run from a terminal to choose one, or register it with drabi game add"
    );
    for (index, candidate) in candidates.iter().enumerate() {
        eprintln!(
            "{}. {} — {}",
            index + 1,
            candidate.name,
            candidate.path.display()
        );
        if !candidate.arguments.is_empty() {
            eprintln!("   Arguments: {:?}", candidate.arguments);
        }
    }
    eprint!("Enter a number or the absolute path to the installed .exe (empty to cancel): ");
    io::stderr().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    ensure!(!input.is_empty(), "Registration cancelled");
    if let Ok(index) = input.parse::<usize>() {
        ensure!(
            index > 0 && index <= candidates.len(),
            "Invalid executable selection"
        );
        return Ok(candidates.remove(index - 1));
    }
    let path = Path::new(input);
    ensure!(
        path.is_absolute() && is_exe(path) && path.is_file(),
        "Expected an absolute path to an existing .exe"
    );
    Ok(Candidate {
        name: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path: path.canonicalize()?,
        directory: None,
        arguments: Vec::new(),
        registry_key: None,
        version: None,
        publisher: None,
        score: 0,
    })
}

pub fn uninstall(paths: &Paths, id: &str, purge: bool) -> Result<()> {
    let _operation = operation_lock(paths)?;
    let config = paths.game_path(id)?;
    let raw =
        fs::read_to_string(&config).with_context(|| format!("Reading {}", config.display()))?;
    let file = crate::config::parse_game(&raw, id)?;
    let prefix = launch::prefix_path(
        paths,
        file.prefix.as_ref().map(|prefix| prefix.path.as_path()),
    )?;
    launch::check_prefix(&prefix)?;
    let prefix = prefix.canonicalize()?;
    let root = wine_root(&prefix);
    let exe = paths.absolute(&file.game.exe)?;
    let existed = exe.is_file();
    let canonical_exe = exe.canonicalize().unwrap_or_else(|_| exe.clone());
    let entries = registry(&root)?;
    let (key, entry) = file
        .installation
        .as_ref()
        .and_then(|install| install.registry_key.as_ref())
        .and_then(|key| entries.get_key_value(key))
        .filter(|(_, entry)| entry.uninstall.is_some())
        .or_else(|| find_uninstall(&root, &canonical_exe, &entries))
        .context(
            "No unambiguous registered Windows uninstaller was found. The game has been kept",
        )?;
    let command = entry
        .uninstall
        .as_deref()
        .context("The registry entry has no uninstaller")?;
    let (uninstaller, arguments) = uninstall_command(&root, command)?;
    let proton = runner::resolve(
        paths,
        file.runner.as_ref().map(|runner| runner.name.as_str()),
    )?;
    let owner = if purge {
        Some(purge_owner(paths, id, &prefix)?)
    } else {
        None
    };
    let action = if purge {
        "Uninstall and delete its dedicated prefix, including saves"
    } else {
        "Run the registered uninstaller"
    };
    eprint!(
        "{action} for {} in {}? Type yes to continue: ",
        file.game.name,
        prefix.display()
    );
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    ensure!(answer.trim() == "yes", "Uninstallation cancelled");

    let status = launch::run(
        paths,
        uninstaller
            .to_str()
            .context("Uninstaller path is not UTF-8")?,
        launch::Options {
            prefix: Some(prefix.clone()),
            runner: Some(
                proton
                    .to_str()
                    .context("Proton path is not UTF-8")?
                    .to_owned(),
            ),
            arguments,
            ..Default::default()
        },
    )?;
    ensure!(
        status == 0,
        "Uninstaller exited with status {status}. The registration and shortcut were kept"
    );
    let remaining = registry(&root)?;
    ensure!(
        !remaining.contains_key(key) || (existed && !exe.is_file()),
        "The uninstaller exited successfully but the application still appears installed. The registration and shortcut were kept"
    );

    let _config_lock = paths.lock_config()?;
    ensure!(
        fs::read_to_string(&config)? == raw,
        "The game configuration changed during uninstallation. Review it before removing the registration"
    );
    if let Some(owner) = owner {
        // A second check under the configuration lock prevents a newly registered game losing its prefix.
        purge_owner(paths, id, &prefix)?;
        ensure!(
            remaining
                .keys()
                .all(|key| owner.initial_registry_keys.contains(key)),
            "The prefix contains additional registered applications. Its files and the game registration were kept"
        );
        fs::remove_dir_all(&prefix)
            .with_context(|| format!("Removing dedicated prefix {}", prefix.display()))?;
    }
    desktop::remove_if_present(paths, id)?;
    fs::remove_file(config)?;
    crate::artwork::remove(paths, id);
    crate::lifecycle::forget_recent(paths, id);
    println!("Uninstalled {}", file.game.name);
    Ok(())
}

fn uninstall_command(root: &Path, command: &str) -> Result<(PathBuf, Vec<String>)> {
    let argv = winsplit::split(command);
    let first = argv
        .first()
        .context("The registry uninstaller command is empty")?;
    let name = first.rsplit(['\\', '/']).next().unwrap_or(first);
    let (path, args) =
        if name.eq_ignore_ascii_case("msiexec.exe") || name.eq_ignore_ascii_case("msiexec") {
            let mut args = argv[1..].to_vec();
            normalize_msi(&mut args)?;
            (windows_path(root, r"C:\windows\system32\msiexec.exe"), args)
        } else if let Some(path) = windows_path(root, first) {
            (Some(path), argv[1..].to_vec())
        } else if !command.trim_start().starts_with('"') {
            let lower = command.to_ascii_lowercase();
            let candidate = lower.match_indices(".exe").find_map(|(index, _)| {
                let end = index + 4;
                let (path, tail) = command.split_at(end);
                if !tail.is_empty() && !tail.starts_with(char::is_whitespace) {
                    return None;
                }
                Some((windows_path(root, path)?, winsplit::split(tail)))
            });
            candidate.map_or((None, Vec::new()), |(path, args)| (Some(path), args))
        } else {
            (None, Vec::new())
        };
    let path = path
        .context("The registry uninstaller could not be resolved to a local .exe in this prefix")?;
    let drive = drive_root(root)?;
    ensure!(
        path.is_file() && is_exe(&path) && path.starts_with(drive),
        "The registry uninstaller is outside the prefix or is not an executable"
    );
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    ensure!(
        ![
            "cmd",
            "powershell",
            "pwsh",
            "rundll32",
            "wscript",
            "cscript",
            "explorer",
            "mshta",
            "regsvr32",
            "start"
        ]
        .contains(&name.as_str()),
        "The registry entry requires a Windows command wrapper. Run the uninstaller manually"
    );
    Ok((path, args))
}

fn normalize_msi(args: &mut Vec<String>) -> Result<()> {
    let operation = args
        .first()
        .context("The MSI uninstaller has no product code")?
        .clone();
    let (flag, rest) = operation
        .split_at_checked(2)
        .context("Unsupported MSI uninstall command")?;
    ensure!(
        matches!(flag.to_ascii_lowercase().as_str(), "/i" | "/x"),
        "Unsupported MSI uninstall command"
    );
    let code = if rest.is_empty() {
        args.get(1)
            .context("The MSI uninstaller has no product code")?
            .as_str()
    } else {
        rest
    };
    let code = code
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .context("MSI uninstall requires a registered product GUID")?;
    ensure!(
        code.len() == 36
            && code
                .bytes()
                .enumerate()
                .all(|(index, byte)| if [8, 13, 18, 23].contains(&index) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }),
        "Invalid MSI product GUID"
    );
    // The registry can contain a maintenance (/I) command. Request removal explicitly.
    *args = vec!["/X".into(), format!("{{{code}}}")];
    Ok(())
}

fn purge_owner(paths: &Paths, id: &str, prefix: &Path) -> Result<Owner> {
    let parent = isolated_root(paths)?
        .canonicalize()
        .context("Only a Drabi-owned isolated prefix can be purged")?;
    ensure!(
        prefix.parent() == Some(parent.as_path()),
        "Only a Drabi-owned isolated prefix can be purged"
    );
    let marker = prefix.join(OWNER);
    ensure!(
        !marker.is_symlink(),
        "The isolated prefix ownership marker is invalid"
    );
    let owner: Owner = serde_json::from_str(
        &fs::read_to_string(marker).context("Missing isolated prefix ownership marker")?,
    )?;
    ensure!(
        owner.game_id == id,
        "The selected prefix is not owned by {id}"
    );
    let overlaps = |path: PathBuf| {
        let canonical = path.canonicalize().unwrap_or(path);
        canonical.starts_with(prefix) || prefix.starts_with(&canonical)
    };
    ensure!(
        !overlaps(launch::prefix_path(paths, None)?),
        "The selected prefix is the default or overlaps it"
    );
    for other_id in crate::game_ids(paths)? {
        if other_id == id {
            continue;
        }
        let other = paths.load_game(&other_id)?;
        let other_prefix = launch::prefix_path(
            paths,
            other.prefix.as_ref().map(|prefix| prefix.path.as_path()),
        )?;
        ensure!(
            !overlaps(other_prefix)
                && !paths
                    .absolute(&other.game.exe)?
                    .canonicalize()
                    .is_ok_and(|exe| exe.starts_with(prefix)),
            "The prefix is shared with {other_id}"
        );
    }
    Ok(owner)
}

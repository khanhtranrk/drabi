use crate::{config::Paths, runner, runtime::curl};
use anyhow::{Context, Result, ensure};
use clap::ValueEnum;
use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256, Sha512};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};
use xz2::read::XzDecoder;

const MARKER: &str = ".drabi-managed";

#[derive(Clone, Copy, ValueEnum)]
pub enum Source {
    Ge,
    Dw,
    Umu,
    Valve,
}

const SOURCES: [Source; 4] = [Source::Ge, Source::Dw, Source::Umu, Source::Valve];

impl Source {
    fn repo(self) -> Option<&'static str> {
        match self {
            Self::Ge => Some("GloriousEggroll/proton-ge-custom"),
            Self::Dw => Some("dawn-winery/dwproton-mirror"),
            Self::Umu => Some("Open-Wine-Components/umu-proton"),
            Self::Valve => None,
        }
    }

    fn family(self) -> &'static str {
        match self {
            Self::Ge => "GE-Proton",
            Self::Dw => "DW-Proton",
            Self::Umu => "UMU-Proton",
            Self::Valve => "Valve Proton",
        }
    }
}

const STEAM_TOOLS: [(&str, u32, &str); 5] = [
    ("proton-11.0", 4628710, "Proton 11.0"),
    ("proton-10.0", 3658110, "Proton 10.0"),
    ("proton-9.0", 2805730, "Proton 9.0"),
    ("proton-8.0", 2348590, "Proton 8.0"),
    ("proton-experimental", 1493710, "Proton - Experimental"),
];

pub fn steam_runner(paths: &Paths, name: &str) -> Option<PathBuf> {
    let (_, _, directory) = STEAM_TOOLS
        .iter()
        .find(|(id, _, _)| id.eq_ignore_ascii_case(name))?;
    [
        paths.data.with_file_name("Steam"),
        paths.home.join(".local/share/Steam"),
        paths.home.join(".steam/root"),
        paths
            .home
            .join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ]
    .into_iter()
    .map(|root| root.join("steamapps/common").join(directory))
    .find(|path| runner::executable(&path.join("proton")))
}

pub fn steam_runners(paths: &Paths) -> impl Iterator<Item = (&'static str, PathBuf)> + '_ {
    STEAM_TOOLS
        .iter()
        .filter_map(|(id, _, _)| steam_runner(paths, id).map(|path| (*id, path)))
}

fn open_steam(app: u32) -> Result<()> {
    let status = std::process::Command::new("xdg-open")
        .arg(format!("steam://install/{app}"))
        .status()
        .context("Opening the Steam install link with xdg-open")?;
    ensure!(
        status.success(),
        "Could not open Steam. Install and sign in to Steam first"
    );
    println!("Opened the Steam install request. Confirm the download in Steam.");
    Ok(())
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
    prerelease: bool,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

impl Release {
    fn checksum(&self, archive: &Asset) -> Option<&Asset> {
        let stem = archive
            .name
            .strip_suffix(".tar.gz")
            .or_else(|| archive.name.strip_suffix(".tar.xz"))?;
        let name = format!("{stem}.sha512sum");
        self.assets.iter().find(|asset| asset.name == name)
    }

    fn archive(&self) -> Option<&Asset> {
        let arch = std::env::consts::ARCH;
        for suffix in [".tar.gz", ".tar.xz"] {
            let named = format!("{}-{arch}{suffix}", self.tag_name);
            let legacy = format!("{}{suffix}", self.tag_name);
            if let Some(asset) = self
                .assets
                .iter()
                .find(|asset| asset.name == named || (arch == "x86_64" && asset.name == legacy))
            {
                return Some(asset);
            }
        }
        None
    }
}

fn metadata<T: serde::de::DeserializeOwned>(url: &str) -> Result<T> {
    let output = curl()
        .arg(url)
        .output()
        .context("Querying GitHub for Proton releases with curl")?;
    ensure!(
        output.status.success(),
        "GitHub release query failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    serde_json::from_slice(&output.stdout).context("Reading Proton release metadata")
}

pub fn available(paths: &Paths, source: Option<Source>, limit: usize) -> Result<()> {
    if let Some(source) = source {
        return search(paths, source, None, limit).map(|_| ());
    }
    let mut found = false;
    for source in SOURCES {
        println!("{}", source.family());
        match search(paths, source, None, limit) {
            Ok(count) => found |= count > 0,
            Err(error) => eprintln!("warning: {}: {error:#}", source.family()),
        }
    }
    ensure!(found, "No Proton versions could be listed");
    Ok(())
}

pub fn search(paths: &Paths, source: Source, query: Option<&str>, limit: usize) -> Result<usize> {
    let query = query.unwrap_or("").to_ascii_lowercase();
    let Some(repo) = source.repo() else {
        let rows = STEAM_TOOLS
            .iter()
            .filter(|(id, _, label)| {
                id.contains(&query) || label.to_ascii_lowercase().contains(&query)
            })
            .take(limit);
        let mut count = 0;
        for (id, app, _) in rows {
            let state = if steam_runner(paths, id).is_some() {
                "installed"
            } else {
                "available"
            };
            println!("{id}\t{state}\tSteam (app {app})");
            count += 1;
        }
        return Ok(count);
    };
    let releases: Vec<Release> = metadata(&format!(
        "https://api.github.com/repos/{repo}/releases?per_page=100"
    ))?;
    let installed = runner::scan(paths)?;
    let mut count = 0;
    for release in releases
        .iter()
        .filter(|release| {
            !release.prerelease
                && release.archive().is_some_and(|asset| {
                    asset
                        .digest
                        .as_deref()
                        .is_some_and(|value| value.starts_with("sha256:"))
                        || release.checksum(asset).is_some()
                })
                && release.tag_name.to_ascii_lowercase().contains(&query)
        })
        .take(limit)
    {
        let state = if installed.contains_key(&release.tag_name) {
            "installed"
        } else {
            "available"
        };
        println!("{}\t{state}", release.tag_name);
        count += 1;
    }
    if count == 0 {
        println!("No matching releases among the latest 100.");
    }
    Ok(count)
}

fn root(paths: &Paths) -> PathBuf {
    paths.data.join("runners/proton")
}

pub fn is_managed(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
        && fs::symlink_metadata(path.join(MARKER)).is_ok_and(|meta| meta.is_file())
}

fn lock(paths: &Paths) -> Result<File> {
    fs::create_dir_all(root(paths))?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root(paths).join(".lock"))?;
    lock.lock()?;
    Ok(lock)
}

fn version(name: &str) -> Result<Source> {
    ensure!(
        !name.is_empty()
            && name.len() <= 80
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.')),
        "Invalid Proton version"
    );
    let lower = name.to_ascii_lowercase();
    for (prefix, source) in [
        ("ge-proton", Source::Ge),
        ("dwproton-", Source::Dw),
        ("umu-proton-", Source::Umu),
    ] {
        if lower.starts_with(prefix) && lower.len() > prefix.len() {
            return Ok(source);
        }
    }
    anyhow::bail!("Use an exact version shown by drabi proton list --available")
}

fn family(name: &str) -> Option<Source> {
    match name.to_ascii_lowercase().as_str() {
        "latest" | "ge" | "ge-proton" => Some(Source::Ge),
        "dw" | "dw-proton" => Some(Source::Dw),
        "umu" | "umu-proton" => Some(Source::Umu),
        "valve" | "valve-proton" => Some(Source::Valve),
        _ => None,
    }
}

pub fn install(paths: &Paths, name: &str) -> Result<()> {
    let selected = family(name);
    let steam = STEAM_TOOLS
        .iter()
        .find(|(id, _, _)| id.eq_ignore_ascii_case(name))
        .or_else(|| matches!(selected, Some(Source::Valve)).then(|| &STEAM_TOOLS[0]));
    if let Some((id, app, _)) = steam {
        if let Some(path) = steam_runner(paths, id) {
            println!("{id} is already installed at {}", path.display());
            return Ok(());
        }
        return open_steam(*app);
    }
    let source = match selected {
        Some(source) => source,
        None => version(name)?,
    };
    install_release(paths, source, selected.is_none().then_some(name))
}

fn install_release(paths: &Paths, source: Source, tag: Option<&str>) -> Result<()> {
    let repo = source
        .repo()
        .context("Steam manages Valve Proton updates")?;
    let endpoint = tag.map_or_else(|| "latest".to_owned(), |tag| format!("tags/{tag}"));
    let release: Release = metadata(&format!(
        "https://api.github.com/repos/{repo}/releases/{endpoint}"
    ))?;
    ensure!(
        matches!(version(&release.tag_name)?.repo(), Some(found) if found == repo),
        "Unexpected Proton release family"
    );
    ensure!(
        tag.is_none_or(|tag| tag == release.tag_name),
        "Unexpected Proton release tag"
    );
    ensure!(!release.prerelease, "Use a stable Proton release");
    let asset = release
        .archive()
        .with_context(|| format!("No Proton archive for {}", std::env::consts::ARCH))?;
    let expected_url = asset_url(repo, &release, asset)?;

    let _lock = lock(paths)?;
    let target = root(paths).join(&release.tag_name);
    if fs::symlink_metadata(&target).is_ok() {
        ensure!(
            is_managed(&target) && runner::executable(&target.join("proton")),
            "{} already exists. Move or inspect it before installing",
            target.display()
        );
        println!("{} is already installed", release.tag_name);
        return Ok(());
    }
    ensure!(
        !paths
            .settings()?
            .external_runners
            .contains_key(&release.tag_name),
        "An external runner already uses {}",
        release.tag_name
    );

    let stage = tempfile::tempdir_in(root(paths))?;
    let archive = stage.path().join("proton.tar.gz");
    eprintln!("Downloading {}…", release.tag_name);
    let status = curl()
        .args(["--max-time", "900", "--output"])
        .arg(&archive)
        .arg(expected_url)
        .status()?;
    ensure!(status.success(), "Proton download failed ({status})");
    verify(repo, &release, asset, &archive)?;

    let extracted = stage.path().join("extracted");
    fs::create_dir(&extracted)?;
    let compressed = File::open(&archive)?;
    let reader: Box<dyn io::Read> = if asset.name.ends_with(".tar.xz") {
        Box::new(XzDecoder::new(compressed))
    } else {
        Box::new(GzDecoder::new(compressed))
    };
    tar::Archive::new(reader).unpack(&extracted)?;
    let mut entries = fs::read_dir(&extracted)?;
    let directory = entries.next().context("Proton archive is empty")??;
    ensure!(
        entries.next().is_none() && directory.file_type()?.is_dir(),
        "Expected one top-level Proton directory"
    );
    ensure!(
        runner::executable(&directory.path().join("proton")),
        "Proton archive has no executable proton script"
    );

    let _config_lock = paths.lock_config()?;
    ensure!(
        !paths
            .settings()?
            .external_runners
            .contains_key(&release.tag_name),
        "An external runner already uses {}",
        release.tag_name
    );
    fs::write(directory.path().join(MARKER), &release.tag_name)?;
    fs::rename(directory.path(), &target)?;
    println!("Installed {} at {}", release.tag_name, target.display());
    Ok(())
}

fn asset_url(repo: &str, release: &Release, asset: &Asset) -> Result<String> {
    let url = format!(
        "https://github.com/{repo}/releases/download/{}/{}",
        release.tag_name, asset.name
    );
    ensure!(
        asset.browser_download_url == url,
        "Unexpected Proton download URL"
    );
    Ok(url)
}

fn verify(repo: &str, release: &Release, asset: &Asset, archive: &Path) -> Result<()> {
    let mut contents = File::open(archive)?;
    if let Some(expected) = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
    {
        let mut hasher = Sha256::new();
        io::copy(&mut contents, &mut hasher)?;
        ensure!(
            format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected),
            "Proton archive SHA-256 mismatch"
        );
        return Ok(());
    }

    let checksum = release
        .checksum(asset)
        .context("Proton release has no supported checksum")?;
    let url = asset_url(repo, release, checksum)?;
    let output = curl()
        .arg(url)
        .output()
        .context("Downloading the Proton checksum")?;
    ensure!(output.status.success(), "Proton checksum download failed");
    let text = std::str::from_utf8(&output.stdout)?;
    let mut fields = text.split_whitespace();
    let expected = fields.next().context("Proton checksum is empty")?;
    ensure!(
        expected.len() == 128 && expected.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid GE-Proton SHA-512 checksum"
    );
    if let Some(name) = fields.next() {
        ensure!(
            name.trim_start_matches('*') == asset.name,
            "Proton checksum names a different archive"
        );
    }
    let mut hasher = Sha512::new();
    io::copy(&mut contents, &mut hasher)?;
    ensure!(
        format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected),
        "Proton archive SHA-512 mismatch"
    );
    Ok(())
}

pub fn update(paths: &Paths, name: &str) -> Result<()> {
    let source = match family(name) {
        Some(source) => source,
        None => {
            let source = version(name)?;
            ensure!(
                is_managed(&root(paths).join(name)),
                "{name} is not managed by drabi"
            );
            source
        }
    };
    install_release(paths, source, None)?;
    println!("Existing runner selections still point to their saved versions.");
    Ok(())
}

pub fn remove(paths: &Paths, name: &str) -> Result<()> {
    version(name)?;
    let _lock = lock(paths)?;
    let _config_lock = paths.lock_config()?;
    let path = root(paths).join(name);
    ensure!(
        fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir()) && is_managed(&path),
        "{name} is not managed by drabi"
    );
    let canonical = path.canonicalize()?;
    let settings = paths.settings()?;
    let references = |selection: &str| {
        selection == name
            || (selection.contains('/')
                && paths
                    .absolute(Path::new(selection))
                    .and_then(|value| Ok(value.canonicalize()?))
                    .is_ok_and(|value| value.starts_with(&canonical)))
    };
    ensure!(
        !settings.default_runner.as_deref().is_some_and(references),
        "{name} is selected as the default runner"
    );
    for (alias, location) in settings.external_runners {
        ensure!(
            !paths
                .absolute(&location)?
                .canonicalize()
                .is_ok_and(|value| value.starts_with(&canonical)),
            "{name} is referenced by external registration {alias}"
        );
    }
    for id in crate::game_ids(paths)? {
        let game = paths.load_game(&id)?;
        ensure!(
            !game
                .runner
                .as_ref()
                .is_some_and(|runner| references(&runner.name)),
            "{name} is selected by game {id}"
        );
    }
    let trash = tempfile::tempdir_in(root(paths))?;
    fs::rename(&path, trash.path().join("runner"))?;
    trash.close()?;
    println!("Removed {name}");
    Ok(())
}

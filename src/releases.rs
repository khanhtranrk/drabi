use crate::{
    config::{Paths, validate_id},
    runner,
    runtime::curl,
};
use anyhow::{Context, Result, ensure};
use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256, Sha512};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

const API: &str = "https://api.github.com/repos/GloriousEggroll/proton-ge-custom/releases";
const ASSET_BASE: &str = "https://github.com/GloriousEggroll/proton-ge-custom/releases/download/";
const MARKER: &str = ".drabi-managed";

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
        let name = format!("{}.sha512sum", archive.name.strip_suffix(".tar.gz")?);
        self.assets.iter().find(|asset| asset.name == name)
    }

    fn archive(&self) -> Option<&Asset> {
        let arch = std::env::consts::ARCH;
        let named = format!("{}-{arch}.tar.gz", self.tag_name);
        let legacy = format!("{}.tar.gz", self.tag_name);
        self.assets
            .iter()
            .find(|asset| asset.name == named)
            .or_else(|| {
                (arch == "x86_64")
                    .then(|| self.assets.iter().find(|asset| asset.name == legacy))
                    .flatten()
            })
    }
}

fn metadata<T: serde::de::DeserializeOwned>(url: &str) -> Result<T> {
    let output = curl()
        .arg(url)
        .output()
        .context("Querying GitHub for GE-Proton with curl")?;
    ensure!(
        output.status.success(),
        "GitHub release query failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    serde_json::from_slice(&output.stdout).context("Reading GE-Proton release metadata")
}

pub fn search(paths: &Paths, query: Option<&str>, limit: usize) -> Result<()> {
    let releases: Vec<Release> = metadata(&format!("{API}?per_page=100"))?;
    let installed = runner::scan(paths)?;
    let query = query.unwrap_or("").to_ascii_lowercase();
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
        println!("No matching GE-Proton releases among the latest 100.");
    }
    Ok(())
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

fn version(name: &str) -> Result<()> {
    validate_id(name)?;
    ensure!(
        name.starts_with("GE-Proton") && name != "GE-Proton",
        "Use a GE-Proton version shown by drabi proton search"
    );
    Ok(())
}

pub fn install(paths: &Paths, name: &str) -> Result<()> {
    let query = if matches!(name, "latest" | "GE-Proton") {
        format!("{API}/latest")
    } else {
        version(name)?;
        format!("{API}/tags/{name}")
    };
    let release: Release = metadata(&query)?;
    version(&release.tag_name)?;
    ensure!(!release.prerelease, "Use a stable GE-Proton release");
    let asset = release
        .archive()
        .with_context(|| format!("No GE-Proton archive for {}", std::env::consts::ARCH))?;
    let expected_url = format!("{ASSET_BASE}{}/{}", release.tag_name, asset.name);
    ensure!(
        asset.browser_download_url == expected_url,
        "Unexpected GE-Proton archive URL"
    );
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
    ensure!(status.success(), "GE-Proton download failed ({status})");
    verify(&release, asset, &archive)?;

    let extracted = stage.path().join("extracted");
    fs::create_dir(&extracted)?;
    tar::Archive::new(GzDecoder::new(File::open(&archive)?)).unpack(&extracted)?;
    let mut entries = fs::read_dir(&extracted)?;
    let directory = entries.next().context("GE-Proton archive is empty")??;
    ensure!(
        entries.next().is_none() && directory.file_type()?.is_dir(),
        "Expected one top-level GE-Proton directory"
    );
    ensure!(
        runner::executable(&directory.path().join("proton")),
        "GE-Proton archive has no executable proton script"
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

fn verify(release: &Release, asset: &Asset, archive: &Path) -> Result<()> {
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
            "GE-Proton archive SHA-256 mismatch"
        );
        return Ok(());
    }

    let checksum = release
        .checksum(asset)
        .context("GE-Proton release has no supported checksum")?;
    let url = format!("{ASSET_BASE}{}/{}", release.tag_name, checksum.name);
    ensure!(
        url == checksum.browser_download_url,
        "Unexpected GE-Proton checksum URL"
    );
    let output = curl()
        .arg(url)
        .output()
        .context("Downloading the GE-Proton checksum")?;
    ensure!(
        output.status.success(),
        "GE-Proton checksum download failed"
    );
    let text = std::str::from_utf8(&output.stdout)?;
    let mut fields = text.split_whitespace();
    let expected = fields.next().context("GE-Proton checksum is empty")?;
    ensure!(
        expected.len() == 128 && expected.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid GE-Proton SHA-512 checksum"
    );
    if let Some(name) = fields.next() {
        ensure!(
            name.trim_start_matches('*') == asset.name,
            "GE-Proton checksum names a different archive"
        );
    }
    let mut hasher = Sha512::new();
    io::copy(&mut contents, &mut hasher)?;
    ensure!(
        format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected),
        "GE-Proton archive SHA-512 mismatch"
    );
    Ok(())
}

pub fn update(paths: &Paths, name: &str) -> Result<()> {
    if name != "GE-Proton" {
        version(name)?;
        ensure!(
            is_managed(&root(paths).join(name)),
            "{name} is not managed by drabi"
        );
    }
    install(paths, "latest")?;
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

use crate::{
    config::{Paths, atomic_write},
    runner::executable,
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const RELEASES: &str =
    "https://api.github.com/repos/Open-Wine-Components/umu-launcher/releases/latest";
const DOWNLOADS: &str = "https://github.com/Open-Wine-Components/umu-launcher/releases/download/";
const CHECK_INTERVAL: Duration = Duration::from_secs(86_400);

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

pub fn entry(paths: &Paths) -> PathBuf {
    paths.data.join("umu/active/umu-run")
}

pub fn ensure_installed(paths: &Paths, force: bool) -> Result<PathBuf> {
    let root = paths.data.join("umu");
    let active = entry(paths);
    if !force && executable(&active) && checked_recently(&root) {
        return Ok(active.canonicalize()?);
    }

    fs::create_dir_all(&root)?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(".install.lock"))?;
    lock.lock()?;
    if !force && executable(&active) && checked_recently(&root) {
        return Ok(active.canonicalize()?);
    }

    let outcome = install_latest(&root);
    if executable(&active) {
        if let Err(error) = atomic_write(&root.join("last-check"), "", true) {
            eprintln!("warning: could not record UMU update check: {error:#}");
        }
        if let Err(error) = outcome {
            if force {
                return Err(error);
            }
            eprintln!("warning: UMU update check failed, using installed version: {error:#}");
        }
        return Ok(active.canonicalize()?);
    }
    outcome?;
    anyhow::bail!("UMU installation did not yield an executable launcher")
}

fn checked_recently(root: &Path) -> bool {
    fs::metadata(root.join("last-check"))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|mtime| mtime.elapsed().ok())
        .is_some_and(|elapsed| elapsed < CHECK_INTERVAL)
}

pub fn curl() -> Command {
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "10",
        "--max-time",
        "60",
    ]);
    command
}

fn install_latest(root: &Path) -> Result<()> {
    let response = curl()
        .arg(RELEASES)
        .output()
        .context("Fetching UMU release metadata with curl")?;
    ensure!(
        response.status.success(),
        "Fetching UMU release metadata failed: {}",
        String::from_utf8_lossy(&response.stderr).trim()
    );
    let release: Release =
        serde_json::from_slice(&response.stdout).context("Reading UMU release metadata")?;
    ensure!(
        !release.tag_name.is_empty()
            && release
                .tag_name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_')),
        "Invalid UMU release tag"
    );
    let version = format!("version-{}", release.tag_name);
    let destination = root.join(&version);
    if !executable(&destination.join("umu-run")) {
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name.ends_with("-zipapp.tar"))
            .context("Official UMU release has no zipapp archive")?;
        let digest = asset
            .digest
            .as_deref()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .context("Official UMU release has no SHA-256 digest for its zipapp")?;
        ensure!(
            asset.browser_download_url.starts_with(DOWNLOADS),
            "Unexpected UMU download URL"
        );

        let stage = tempfile::tempdir_in(root)?;
        let archive = stage.path().join("release.tar");
        let status = curl()
            .arg("--output")
            .arg(&archive)
            .arg(&asset.browser_download_url)
            .status()
            .context("Downloading the official UMU zipapp")?;
        ensure!(status.success(), "UMU download failed ({status})");
        let mut hasher = Sha256::new();
        io::copy(&mut File::open(&archive)?, &mut hasher)?;
        ensure!(
            format!("{:x}", hasher.finalize()) == digest,
            "UMU archive digest mismatch"
        );

        tar::Archive::new(File::open(archive)?).unpack(stage.path())?;
        let extracted = stage.path().join("umu");
        let launcher = extracted.join("umu-run");
        ensure!(
            fs::symlink_metadata(&launcher)?.is_file() && executable(&launcher),
            "UMU archive did not contain an executable umu/umu-run"
        );
        let preflight = Command::new(&launcher).arg("--version").output().context(
            "Starting the UMU zipapp. Check that Python and its required modules are installed",
        )?;
        ensure!(
            preflight.status.success(),
            "UMU zipapp preflight failed ({}): {}",
            preflight.status,
            String::from_utf8_lossy(&preflight.stderr).trim()
        );
        fs::rename(extracted, &destination)?;
    }
    if fs::read_link(root.join("active")).ok().as_deref() != Some(Path::new(&version)) {
        let stage = tempfile::tempdir_in(root)?;
        let link = stage.path().join("active");
        symlink(version, &link)?;
        fs::rename(link, root.join("active"))?;
        eprintln!("UMU {} is active", release.tag_name);
    }
    Ok(())
}

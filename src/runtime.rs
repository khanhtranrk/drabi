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

    let update = || -> Result<()> {
        fs::create_dir_all(&root).with_context(|| format!("Preparing {}", root.display()))?;
        let lock_path = root.join(".install.lock");
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("Opening UMU update lock {}", lock_path.display()))?;
        lock.lock()
            .with_context(|| format!("Locking {}", lock_path.display()))?;
        if !force && executable(&active) && checked_recently(&root) {
            return Ok(());
        }
        let outcome = install_latest(&root);
        if executable(&active)
            && let Err(error) = atomic_write(&root.join("last-check"), "", true)
        {
            eprintln!("warning: could not record UMU update check: {error:#}");
        }
        outcome
    };

    let outcome = update();
    if executable(&active) {
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
        publish_version(root, &extracted, &destination)?;
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

fn publish_version(root: &Path, staged: &Path, destination: &Path) -> Result<()> {
    let publish = || {
        fs::rename(staged, destination)
            .with_context(|| format!("Installing UMU at {}", destination.display()))
    };
    match fs::symlink_metadata(destination) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return publish(),
        Err(error) => {
            return Err(error).with_context(|| format!("Inspecting {}", destination.display()));
        }
        Ok(_) => {}
    }

    let recovery = tempfile::Builder::new()
        .prefix("recovery-")
        .tempdir_in(root)?;
    let backup = recovery.path().join("incomplete");
    fs::rename(destination, &backup).with_context(|| {
        format!(
            "Moving incomplete UMU installation {}",
            destination.display()
        )
    })?;
    // Keep the backup even if restoration fails. It can contain user files.
    let recovery = recovery.keep();
    if let Err(error) = publish() {
        fs::rename(&backup, destination).with_context(|| {
            format!(
                "{error:#}. Restoring incomplete installation from {}",
                backup.display()
            )
        })?;
        let _ = fs::remove_dir(recovery);
        return Err(error);
    }
    eprintln!(
        "warning: preserved incomplete UMU installation at {}",
        backup.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_an_incomplete_version_when_publishing_a_repair() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("version-test");
        let staged = root.path().join("staged");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("old"), "preserve me").unwrap();
        fs::create_dir(&staged).unwrap();
        fs::write(staged.join("umu-run"), "replacement").unwrap();

        publish_version(root.path(), &staged, &destination).unwrap();

        let recovery = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("recovery-")
            })
            .unwrap();
        assert_eq!(
            fs::read_to_string(recovery.join("incomplete/old")).unwrap(),
            "preserve me"
        );
        assert_eq!(
            fs::read_to_string(destination.join("umu-run")).unwrap(),
            "replacement"
        );
    }
}

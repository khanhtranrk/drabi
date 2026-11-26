use crate::config::{GameFile, Paths, atomic_write, validate_id};
use anyhow::{Context, Result, bail, ensure};
use image::ImageFormat;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const API: &str = "https://www.steamgriddb.com/api/v2";
pub const KEY_ENV: &str = "STEAMGRIDDB_API_KEY";
const KINDS: [(&str, &str, ImageFormat); 4] = [
    ("grid", "grids", ImageFormat::Jpeg),
    ("hero", "heroes", ImageFormat::Jpeg),
    ("logo", "logos", ImageFormat::Png),
    ("icon", "icons", ImageFormat::Png),
];

#[derive(Deserialize)]
struct Response<T> {
    success: bool,
    data: Option<T>,
}

#[derive(Deserialize)]
struct Match {
    id: u64,
    name: String,
    release_date: Option<i64>,
}

#[derive(Deserialize)]
struct Asset {
    id: u64,
    url: String,
    width: u32,
    height: u32,
    #[serde(default)]
    score: i64,
}

#[derive(Deserialize, Serialize)]
struct Manifest {
    provider: String,
    game_id: u64,
    assets: BTreeMap<String, SavedAsset>,
}

#[derive(Deserialize, Serialize)]
struct SavedAsset {
    id: u64,
    file: String,
}

pub fn discover(paths: &Paths, game: &GameFile) {
    if let Err(error) = fetch(paths, game, None, false) {
        eprintln!("warning: artwork for {:?}: {error:#}", game.game.name);
    }
}

pub fn fetch(
    paths: &Paths,
    game: &GameFile,
    provider_id: Option<u64>,
    refresh: bool,
) -> Result<()> {
    let dir = directory(paths, &game.game.id)?;
    let saved = read(&dir);
    if !refresh
        && provider_id.is_none()
        && saved.as_ref().is_some_and(|manifest| {
            !manifest.assets.is_empty()
                && KINDS.iter().all(|(kind, ..)| {
                    !manifest.assets.contains_key(*kind)
                        || asset_path(&dir, manifest, kind).is_some()
                })
        })
    {
        return Ok(());
    }
    let key = std::env::var(KEY_ENV).unwrap_or_default();
    if key.is_empty() {
        bail!("SteamGridDB discovery unavailable. Set {KEY_ENV}");
    }
    ensure!(
        key.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "Invalid SteamGridDB API key format"
    );

    let game_id = match provider_id {
        Some(id) => id,
        None => match saved.as_ref() {
            Some(manifest) => manifest.game_id,
            None => {
                let matches: Vec<Match> = query(
                    &key,
                    &format!("search/autocomplete/{}", encode(&game.game.name)),
                )?;
                select(&game.game.name, &matches)?
            }
        },
    };
    let staging = tempfile::tempdir()?;
    let mut manifest = Manifest {
        provider: "steamgriddb".into(),
        game_id,
        assets: BTreeMap::new(),
    };
    for (kind, endpoint, format) in KINDS {
        let old = saved
            .as_ref()
            .filter(|entry| entry.game_id == game_id)
            .and_then(|entry| Some((entry.assets.get(kind)?, asset_path(&dir, entry, kind)?)));
        let result = download_kind(
            &key,
            game_id,
            kind,
            endpoint,
            format,
            staging.path(),
            old.as_ref().map(|(asset, _)| asset.id),
        );
        match result {
            Ok(Some(asset)) => {
                manifest.assets.insert(kind.into(), asset);
            }
            outcome => {
                if let Err(error) = outcome {
                    eprintln!("warning: SteamGridDB {kind}: {error:#}");
                }
                if let Some((asset, _)) = old {
                    manifest.assets.insert(
                        kind.into(),
                        SavedAsset {
                            id: asset.id,
                            file: asset.file.clone(),
                        },
                    );
                }
            }
        }
    }
    ensure!(
        !manifest.assets.is_empty(),
        "No downloadable SteamGridDB artwork found for game {game_id}"
    );

    let _lock = paths.lock_config()?;
    let current = paths.load_game(&game.game.id)?;
    ensure!(
        current.game.name == game.game.name && current.game.exe == game.game.exe,
        "Game registration changed while downloading artwork"
    );
    let dir = directory(paths, &game.game.id)?;
    fs::create_dir_all(&dir)?;
    for asset in manifest.assets.values() {
        let source = staging.path().join(&asset.file);
        if source.is_file() {
            crate::config::atomic_write_bytes(&dir.join(&asset.file), &fs::read(source)?, true)?;
        }
    }
    manifest
        .assets
        .retain(|kind, asset| local_file(&dir, kind, asset).is_some());
    ensure!(
        !manifest.assets.is_empty(),
        "Artwork changed during download. Run game artwork again"
    );
    atomic_write(
        &dir.join("steamgriddb.json"),
        &serde_json::to_string_pretty(&manifest)?,
        true,
    )?;
    println!(
        "Saved artwork for {} (SteamGridDB #{game_id})",
        game.game.name
    );
    Ok(())
}

fn directory(paths: &Paths, id: &str) -> Result<PathBuf> {
    validate_id(id)?;
    let root = paths.data.join("artwork");
    let dir = root.join(id);
    for path in [&root, &dir] {
        match fs::symlink_metadata(path) {
            Ok(meta) => ensure!(
                meta.is_dir(),
                "Expected artwork directory at {}",
                path.display()
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(dir)
}

fn read(dir: &Path) -> Option<Manifest> {
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(dir.join("steamgriddb.json")).ok()?).ok()?;
    (manifest.provider == "steamgriddb").then_some(manifest)
}

fn file_name(kind: &str, id: u64) -> String {
    let ext = if matches!(kind, "grid" | "hero") {
        "jpg"
    } else {
        "png"
    };
    format!("{kind}-{id}.{ext}")
}

fn local_file(dir: &Path, kind: &str, asset: &SavedAsset) -> Option<PathBuf> {
    let file = file_name(kind, asset.id);
    if asset.file != file {
        return None;
    }
    let path = dir.join(file);
    fs::symlink_metadata(&path).ok()?.is_file().then_some(path)
}

fn asset_path(dir: &Path, manifest: &Manifest, kind: &str) -> Option<PathBuf> {
    local_file(dir, kind, manifest.assets.get(kind)?)
}

pub fn local(paths: &Paths, id: &str) -> BTreeMap<String, PathBuf> {
    let mut assets = BTreeMap::new();
    if let Ok(dir) = directory(paths, id)
        && let Some(manifest) = read(&dir)
    {
        for (kind, ..) in KINDS {
            if let Some(path) =
                asset_path(&dir, &manifest, kind).filter(|path| path.to_str().is_some())
            {
                assets.insert(kind.into(), path);
            }
        }
    }
    assets
}

// Call with the configuration lock after removing the registration. Each
// directory is owned by one ID, even when games share a SteamGridDB match.
pub fn remove(paths: &Paths, id: &str) {
    let result = remove_owned(paths, id);
    if let Err(error) = result {
        eprintln!("warning: could not remove artwork for {id:?}: {error:#}");
    }
}

fn remove_owned(paths: &Paths, id: &str) -> Result<()> {
    let dir = directory(paths, id)?;
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let managed = name.to_str().is_some_and(|name| {
            KINDS.iter().any(|(kind, ..)| {
                name.strip_prefix(kind)
                    .and_then(|tail| tail.strip_prefix('-'))
                    .and_then(|tail| tail.split_once('.'))
                    .and_then(|(id, _)| id.parse::<u64>().ok())
                    .is_some_and(|id| file_name(kind, id) == name)
            })
        });
        if managed || name == "steamgriddb.json" {
            fs::remove_file(entry.path())?;
        }
    }
    match fs::remove_dir(dir) {
        Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(()),
        result => Ok(result?),
    }
}

fn curl() -> Command {
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--fail",
        "--silent",
        "--proto",
        "=https",
        "--connect-timeout",
        "4",
        "--max-time",
        "10",
        "--max-filesize",
        "12582912",
    ]);
    command.env_remove(KEY_ENV);
    command
}

fn query<T: DeserializeOwned>(key: &str, route: &str) -> Result<T> {
    // Supply the credential over stdin so it stays out of process arguments.
    // API requests use the fixed origin and never follow redirects.
    let mut child = curl()
        .args(["--config", "-", "--url"])
        .arg(format!("{API}/{route}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("Starting curl")?;
    let written = writeln!(
        child.stdin.take().context("Opening curl input")?,
        "header = \"Authorization: Bearer {key}\""
    );
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "SteamGridDB API request failed ({})",
        output.status
    );
    written?;
    let response: Response<T> =
        serde_json::from_slice(&output.stdout).context("Invalid SteamGridDB response")?;
    ensure!(response.success, "SteamGridDB rejected the request");
    response.data.context("SteamGridDB returned no data")
}

fn download_kind(
    key: &str,
    game_id: u64,
    kind: &str,
    endpoint: &str,
    format: ImageFormat,
    dir: &Path,
    old_id: Option<u64>,
) -> Result<Option<SavedAsset>> {
    let mut assets: Vec<Asset> = query(
        key,
        &format!("{endpoint}/game/{game_id}?types=static&nsfw=false&humor=false"),
    )?;
    assets.retain(|asset| {
        asset.height > 0
            && match kind {
                "grid" => asset.height > asset.width,
                "hero" => asset.width > asset.height,
                _ => true,
            }
    });
    assets.sort_by_key(|asset| std::cmp::Reverse(asset.score));
    for asset in assets.into_iter().take(3) {
        let file = file_name(kind, asset.id);
        if old_id == Some(asset.id) {
            return Ok(Some(SavedAsset { id: asset.id, file }));
        }
        if ![
            "https://cdn.steamgriddb.com/",
            "https://cdn2.steamgriddb.com/",
        ]
        .iter()
        .any(|host| asset.url.starts_with(host))
        {
            continue;
        }
        let response = curl()
            .arg("--url")
            .arg(&asset.url)
            .output()
            .context("Downloading artwork")?;
        if response.status.success()
            && save_image(&response.stdout, &dir.join(&file), kind, format).is_ok()
        {
            return Ok(Some(SavedAsset { id: asset.id, file }));
        }
    }
    Ok(None)
}

fn save_image(bytes: &[u8], path: &Path, kind: &str, format: ImageFormat) -> Result<()> {
    let mut reader = image::ImageReader::new(io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let (width, height) = match kind {
        "grid" => (800, 1200),
        "hero" => (1920, 1080),
        "logo" => (1000, 400),
        _ => (512, 512),
    };
    let image = reader.decode()?.thumbnail(width, height);
    let mut encoded = io::Cursor::new(Vec::new());
    if format == ImageFormat::Jpeg {
        image::DynamicImage::ImageRgb8(image.to_rgb8()).write_to(&mut encoded, format)?;
    } else {
        image.write_to(&mut encoded, format)?;
    }
    fs::write(path, encoded.into_inner())?;
    Ok(())
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn select(name: &str, matches: &[Match]) -> Result<u64> {
    let normalized = |text: &str| {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let name = normalized(name);
    let exact: Vec<_> = matches
        .iter()
        .filter(|item| normalized(&item.name) == name)
        .collect();
    if let [game] = exact.as_slice() {
        return Ok(game.id);
    }
    let candidates: Vec<_> = if exact.is_empty() {
        matches.iter().take(10).collect()
    } else {
        exact.into_iter().take(10).collect()
    };
    ensure!(!candidates.is_empty(), "No matching SteamGridDB game");
    ensure!(
        io::stdin().is_terminal() && io::stderr().is_terminal(),
        "No unique title match. Use game artwork in a terminal or specify --provider-id"
    );
    eprintln!("Select a SteamGridDB title (Enter skips):");
    for (index, game) in candidates.iter().enumerate() {
        eprintln!(
            "  {}. {:?} (#{}{})",
            index + 1,
            game.name,
            game.id,
            if game.release_date.unwrap_or_default() > 0 {
                let date = game.release_date.unwrap_or_default();
                format!(", approx. {}", 1970 + date / 31_556_952)
            } else {
                String::new()
            }
        );
    }
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let index = input
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|index| index.checked_sub(1));
    index
        .and_then(|index| candidates.get(index))
        .map(|game| game.id)
        .context("Artwork selection skipped")
}

use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};
use walkdir::{DirEntry, WalkDir};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct RegistryEntry {
    pub name: Option<String>,
    pub publisher: Option<String>,
    pub version: Option<String>,
    pub location: Option<String>,
    pub icon: Option<String>,
    pub uninstall: Option<String>,
}

pub(super) type Registry = BTreeMap<String, RegistryEntry>;

pub(super) fn wine_root(prefix: &Path) -> PathBuf {
    if prefix.join("pfx").is_dir() && !prefix.join("pfx").is_symlink() {
        prefix.join("pfx")
    } else {
        prefix.to_owned()
    }
}

pub(super) fn registry(root: &Path) -> Result<Registry> {
    let mut entries = BTreeMap::new();
    for hive in ["system.reg", "user.reg"] {
        let path = root.join(hive);
        match fs::read_to_string(&path) {
            Ok(content) => parse_registry(hive, &content, &mut entries),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("Reading {}", path.display())),
        }
    }
    Ok(entries)
}

fn parse_registry(hive: &str, content: &str, entries: &mut Registry) {
    const UNINSTALL: &str = "software\\microsoft\\windows\\currentversion\\uninstall\\";
    let wow64 = UNINSTALL.replacen("software\\", "software\\wow6432node\\", 1);
    let mut key = None;
    for line in content.lines().map(str::trim) {
        if let Some(section) = line.strip_prefix('[') {
            key = section.split_once(']').and_then(|(section, _)| {
                let section = section.replace("\\\\", "\\").to_ascii_lowercase();
                let id = section
                    .strip_prefix(UNINSTALL)
                    .or_else(|| section.strip_prefix(&wow64))?;
                (!id.is_empty() && !id.contains('\\')).then(|| format!("{hive}:{section}"))
            });
            continue;
        }
        let Some(key) = &key else { continue };
        let Some((name, raw)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = wine_string(raw) else {
            continue;
        };
        let entry = entries.entry(key.clone()).or_default();
        match name.trim_matches('"').to_ascii_lowercase().as_str() {
            "displayname" => entry.name = Some(value),
            "publisher" => entry.publisher = Some(value),
            "displayversion" => entry.version = Some(value),
            "installlocation" => entry.location = Some(value),
            "displayicon" => entry.icon = Some(value),
            "uninstallstring" => entry.uninstall = Some(value),
            _ => {}
        }
    }
}

fn wine_string(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("str(2):").unwrap_or(raw);
    let mut chars = raw.strip_prefix('"')?.strip_suffix('"')?.chars();
    let mut value = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next()? {
                '\\' => value.push('\\'),
                '"' => value.push('"'),
                'n' => value.push('\n'),
                other => {
                    value.push('\\');
                    value.push(other);
                }
            }
        } else {
            value.push(ch);
        }
    }
    (!value.trim().is_empty() && !value.contains('\0')).then_some(value)
}

pub(super) fn is_exe(path: &Path) -> bool {
    extension(path, "exe")
}

pub(super) fn drive_root(root: &Path) -> Result<PathBuf> {
    let drive = root
        .join("drive_c")
        .canonicalize()
        .context("Resolving the prefix C: drive")?;
    ensure!(
        drive.starts_with(root.canonicalize()?),
        "The prefix C: drive points outside the prefix"
    );
    Ok(drive)
}

fn extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

/// Resolve local Windows paths without following Wine's host drive mappings.
pub(super) fn windows_path(root: &Path, value: &str) -> Option<PathBuf> {
    let value = value.trim().trim_matches('"').replace('\\', "/");
    let (drive, tail) = value.split_once('/')?;
    let tail = if drive.eq_ignore_ascii_case("c:") || drive.eq_ignore_ascii_case("%systemdrive%") {
        tail.to_owned()
    } else if drive.eq_ignore_ascii_case("%programfiles%") {
        format!("Program Files/{tail}")
    } else if drive.eq_ignore_ascii_case("%programfiles(x86)%") {
        format!("Program Files (x86)/{tail}")
    } else if ["%windir%", "%systemroot%"]
        .iter()
        .any(|name| drive.eq_ignore_ascii_case(name))
    {
        format!("windows/{tail}")
    } else {
        return None;
    };
    local_path(&root.join("drive_c"), &tail)
}

fn local_path(base: &Path, tail: &str) -> Option<PathBuf> {
    let mut path = base.to_owned();
    for component in tail
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
    {
        if component == ".." || component.contains(':') {
            return None;
        }
        let exact = path.join(component);
        if exact.exists() {
            path = exact;
        } else {
            path = fs::read_dir(&path)
                .ok()?
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(component))
                })?
                .path();
        }
    }
    path.canonicalize().ok()
}

pub(super) fn icon_exe(root: &Path, value: &str) -> Option<PathBuf> {
    let path = match value.rsplit_once(',') {
        Some((path, index)) if index.trim().parse::<i32>().is_ok() => path,
        _ => value,
    };
    windows_path(root, path).filter(|path| is_exe(path))
}

#[derive(Clone, PartialEq, Eq)]
struct Stamp(Option<SystemTime>, u64);

pub(super) struct Snapshot {
    pub registry: Registry,
    files: BTreeMap<PathBuf, Stamp>,
}

fn visit(entry: &DirEntry) -> bool {
    !(entry.depth() == 1
        && entry.file_type().is_dir()
        && entry.file_name().eq_ignore_ascii_case("windows"))
}

fn shortcut_location(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str();
        name.eq_ignore_ascii_case("desktop") || name.eq_ignore_ascii_case("start menu")
    })
}

impl Snapshot {
    pub fn read(root: &Path) -> Result<Self> {
        let drive = drive_root(root)?;
        let mut files = BTreeMap::new();
        for entry in WalkDir::new(&drive).into_iter().filter_entry(visit) {
            let entry = entry.with_context(|| format!("Scanning {}", drive.display()))?;
            let path = entry.path();
            if entry.file_type().is_file()
                && (is_exe(path) || (extension(path, "lnk") && shortcut_location(path)))
            {
                let meta = entry.metadata()?;
                files.insert(entry.into_path(), Stamp(meta.modified().ok(), meta.len()));
            }
        }
        Ok(Self {
            registry: registry(root)?,
            files,
        })
    }

    fn changed_files<'a>(&'a self, before: &'a Self) -> impl Iterator<Item = &'a PathBuf> {
        self.files
            .iter()
            .filter(|(path, stamp)| before.files.get(*path) != Some(*stamp))
            .map(|(path, _)| path)
    }
}

#[derive(Debug, Default)]
struct PeInfo {
    product: Option<String>,
    company: Option<String>,
    description: Option<String>,
    version: Option<String>,
    original: Option<String>,
}

impl PeInfo {
    fn read(path: &Path) -> Option<Self> {
        use pelite::{FileMap, PeFile};

        let map = FileMap::open(path).ok()?;
        let file = PeFile::from_bytes(&map).ok()?;
        let Ok(resources) = file.resources() else {
            return Some(Self::default());
        };
        let Ok(info) = resources.version_info() else {
            return Some(Self::default());
        };
        let field = |name| {
            info.translation().iter().find_map(|language| {
                info.value(*language, name)
                    .filter(|value| !value.trim().is_empty())
            })
        };
        Some(Self {
            product: field("ProductName"),
            company: field("CompanyName"),
            description: field("FileDescription"),
            version: field("ProductVersion"),
            original: field("OriginalFilename"),
        })
    }
}

struct Shortcut {
    target: PathBuf,
    directory: Option<PathBuf>,
    arguments: Vec<String>,
    name: Option<String>,
}

fn read_shortcut(root: &Path, path: &Path) -> Option<Shortcut> {
    let link = lnk::ShellLink::open(path, encoding_rs::WINDOWS_1252).ok()?;
    let strings = link.string_data();
    // Read local LinkInfo directly. Some links have no local target.
    let local = link.link_info().as_ref().and_then(|info| {
        let base = info
            .local_base_path_unicode()
            .as_deref()
            .or(info.local_base_path())?;
        let suffix = info
            .common_path_suffix_unicode()
            .as_deref()
            .unwrap_or(info.common_path_suffix());
        let joined = format!(
            "{}\\{}",
            base.trim_end_matches('\\'),
            suffix.trim_start_matches('\\')
        );
        windows_path(root, &joined)
            .filter(|path| path.is_file())
            .or_else(|| windows_path(root, base))
    });
    let target = local
        .filter(|path| is_exe(path) && path.is_file())
        .or_else(|| {
            let relative = strings.relative_path().as_deref()?;
            windows_path(root, relative).or_else(|| {
                path.parent()?
                    .join(relative.replace('\\', "/"))
                    .canonicalize()
                    .ok()
            })
        })?;
    let directory = strings
        .working_dir()
        .as_deref()
        .and_then(|value| windows_path(root, value))
        .filter(|path| path.is_dir() && path.starts_with(root));
    let arguments = strings
        .command_line_arguments()
        .as_deref()
        .map(winsplit::split)
        .unwrap_or_default();
    Some(Shortcut {
        target,
        directory,
        arguments,
        name: path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_owned),
    })
}

fn auxiliary(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    [
        "unins",
        "uninstall",
        "crash",
        "redist",
        "vc_runtime",
        "msiexec",
        "dxsetup",
        "setup",
        "installshield",
        "update",
        "helper",
        "prereq",
        "reporter",
    ]
    .iter()
    .any(|kind| lower.contains(kind))
}

pub(super) fn registry_match(root: &Path, exe: &Path, entry: &RegistryEntry) -> u8 {
    if entry
        .icon
        .as_deref()
        .and_then(|value| icon_exe(root, value))
        .as_deref()
        == Some(exe)
    {
        return 2;
    }
    u8::from(
        entry
            .location
            .as_deref()
            .and_then(|path| windows_path(root, path))
            .is_some_and(|location| {
                location.is_dir()
                    && location != root.join("drive_c")
                    && !location.file_name().is_some_and(|part| {
                        part.eq_ignore_ascii_case("Program Files")
                            || part.eq_ignore_ascii_case("Program Files (x86)")
                    })
                    && exe.starts_with(location)
            }),
    )
}

fn matched_entry<'a>(
    root: &Path,
    exe: &Path,
    entries: impl Iterator<Item = (&'a String, &'a RegistryEntry)>,
) -> Option<(&'a String, &'a RegistryEntry, u8)> {
    let mut matches: Vec<_> = entries
        .filter_map(|(key, entry)| {
            let score = registry_match(root, exe, entry);
            (score > 0).then_some((key, entry, score))
        })
        .collect();
    matches.sort_by_key(|entry| std::cmp::Reverse(entry.2));
    let first = *matches.first()?;
    (matches.get(1).is_none_or(|entry| entry.2 < first.2)).then_some(first)
}

pub(super) fn find_uninstall<'a>(
    root: &Path,
    exe: &Path,
    registry: &'a Registry,
) -> Option<(&'a String, &'a RegistryEntry)> {
    matched_entry(
        root,
        exe,
        registry
            .iter()
            .filter(|(_, entry)| entry.uninstall.is_some()),
    )
    .map(|(key, entry, _)| (key, entry))
}

pub(super) struct Candidate {
    pub path: PathBuf,
    pub name: String,
    pub directory: Option<PathBuf>,
    pub arguments: Vec<String>,
    pub registry_key: Option<String>,
    pub version: Option<String>,
    pub publisher: Option<String>,
    pub score: i32,
}

pub(super) fn candidates(
    root: &Path,
    before: &Snapshot,
    after: &Snapshot,
    installer: &Path,
) -> Vec<Candidate> {
    use std::collections::BTreeSet;
    let drive = root.join("drive_c").canonicalize().ok();
    let entries = || {
        after
            .registry
            .iter()
            .filter(|(key, value)| before.registry.get(*key) != Some(*value))
    };
    let uninstallers: BTreeSet<_> = entries()
        .filter_map(|(_, entry)| {
            let (path, _) = super::uninstall_command(root, entry.uninstall.as_deref()?).ok()?;
            Some(path)
        })
        .collect();
    let valid_target = |path: &Path| {
        path != installer
            && !uninstallers.contains(path)
            && is_exe(path)
            && path.is_file()
            && drive.as_ref().is_some_and(|drive| {
                path.starts_with(drive)
                    && !path
                        .strip_prefix(drive)
                        .ok()
                        .and_then(|relative| relative.components().next())
                        .is_some_and(|part| part.as_os_str().eq_ignore_ascii_case("windows"))
            })
            && !path
                .file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(auxiliary)
    };
    let mut shortcuts: BTreeMap<PathBuf, Vec<Shortcut>> = BTreeMap::new();
    let mut paths = BTreeSet::new();
    for path in after.changed_files(before) {
        if is_exe(path) {
            paths.insert(path.clone());
        } else if let Some(shortcut) = read_shortcut(root, path)
            && valid_target(&shortcut.target)
        {
            paths.insert(shortcut.target.clone());
            let links = shortcuts.entry(shortcut.target.clone()).or_default();
            if !links.iter().any(|link| {
                link.arguments == shortcut.arguments && link.directory == shortcut.directory
            }) {
                links.push(shortcut);
            }
        }
    }
    for (_, entry) in entries() {
        if let Some(path) = entry.icon.as_deref().and_then(|icon| icon_exe(root, icon)) {
            paths.insert(path);
        }
    }
    let mut candidates = Vec::new();
    for path in paths.into_iter().filter(|path| valid_target(path)) {
        let Some(info) = PeInfo::read(&path) else {
            continue;
        };
        if [&info.original, &info.description]
            .into_iter()
            .flatten()
            .any(|name| auxiliary(name))
        {
            continue;
        }
        let registry = matched_entry(root, &path, entries());
        let links = shortcuts.remove(&path).unwrap_or_default();
        let variants: Vec<_> = if links.is_empty() {
            vec![None]
        } else {
            links.into_iter().map(Some).collect()
        };
        for link in variants {
            let entry = registry.map(|(_, entry, _)| entry);
            let name = entry
                .and_then(|entry| entry.name.clone())
                .or_else(|| info.product.clone())
                .or_else(|| info.description.clone())
                .or_else(|| link.as_ref().and_then(|link| link.name.clone()))
                .unwrap_or_else(|| {
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
            let score = 100 * i32::from(link.is_some())
                + registry.map_or(0, |(_, entry, rank)| {
                    i32::from(rank) * 25
                        + 5 * i32::from(
                            entry
                                .name
                                .as_ref()
                                .zip(info.product.as_ref())
                                .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b)),
                        )
                        + 5 * i32::from(
                            entry
                                .publisher
                                .as_ref()
                                .zip(info.company.as_ref())
                                .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b)),
                        )
                })
                + 5 * i32::from(info.product.is_some());
            let (directory, arguments) = link
                .map(|link| (link.directory, link.arguments))
                .unwrap_or_default();
            candidates.push(Candidate {
                path: path.clone(),
                name,
                directory,
                arguments,
                registry_key: registry.map(|(key, _, _)| key.clone()),
                version: entry
                    .and_then(|entry| entry.version.clone())
                    .or_else(|| info.version.clone()),
                publisher: entry
                    .and_then(|entry| entry.publisher.clone())
                    .or_else(|| info.company.clone()),
                score,
            });
        }
    }
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.score));
    candidates
}

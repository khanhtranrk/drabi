use crate::config::{Paths, atomic_write, atomic_write_bytes};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

pub fn entry_path(paths: &Paths, id: &str) -> Result<PathBuf> {
    paths.game_path(id)?;
    Ok(paths
        .data
        .with_file_name("applications")
        .join(format!("drabi-{id}.desktop")))
}

pub fn create(
    paths: &Paths,
    id: &str,
    name: Option<&str>,
    icon: Option<&Path>,
    dry_run: bool,
) -> Result<()> {
    let file = paths.load_game(id)?;
    let name = name
        .or(file.desktop.name.as_deref())
        .unwrap_or(&file.game.name);
    ensure!(!name.trim().is_empty(), "Shortcut name is required");
    let binary = std::env::current_exe()?;
    let binary = binary
        .to_str()
        .context("drabi executable path must use UTF-8")?;
    ensure!(
        !binary.contains(['\n', '\r', '\t']),
        "Unsupported whitespace in the drabi executable path"
    );
    // Exec has a second escaping layer after the desktop entry string layer.
    let exec = binary
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    // GLib checks the first executable before expanding literal percent codes.
    let prefix = if binary.contains('%') { "env " } else { "" };
    let mut body = format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={prefix}{} run {id}\nTerminal=false\nCategories=Game;\n",
        value(name),
        value(&format!("\"{exec}\""))
    );
    let exe = paths.absolute(&file.game.exe)?;
    let mut extracted = None;
    if let Some(icon) = icon.or(file.desktop.icon.as_deref()) {
        let icon = paths.absolute(icon)?;
        ensure!(icon.is_file(), "Icon not found at {}", icon.display());
        write_icon_key(&mut body, &icon)?;
        if dry_run {
            println!("Icon: {}", icon.display());
        }
    } else {
        match embedded_icon(&exe) {
            Ok(png) => {
                let icon = paths.data.join("icons").join(format!("{id}.png"));
                write_icon_key(&mut body, &icon)?;
                if dry_run {
                    println!("Icon: embedded in {}", exe.display());
                    println!("Generated icon path: {}", icon.display());
                }
                extracted = Some((icon, png));
            }
            Err(error) => {
                if dry_run {
                    println!("Icon: desktop default ({error})");
                }
            }
        }
    }
    let path = entry_path(paths, id)?;
    if dry_run {
        println!("Shortcut: {}\n\n{body}", path.display());
        return Ok(());
    }
    if let Some((icon, png)) = extracted {
        atomic_write_bytes(&icon, &png, true)?;
    }
    atomic_write(&path, &body, true)?;
    println!("Saved {}", path.display());
    Ok(())
}

fn embedded_icon(exe: &Path) -> Result<Vec<u8>> {
    let ico = icoextract_rs::IconExtractor::from_path(exe)?
        .icon_by_index(0)?
        .to_ico_bytes()?;
    let image = image::load_from_memory_with_format(&ico, image::ImageFormat::Ico)?;
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(png.into_inner())
}

fn write_icon_key(body: &mut String, icon: &Path) -> Result<()> {
    body.push_str(&format!(
        "Icon={}\n",
        value(icon.to_str().context("Icon path must use UTF-8")?)
    ));
    Ok(())
}

pub fn remove(paths: &Paths, id: &str) -> Result<()> {
    let path = entry_path(paths, id)?;
    fs::remove_file(&path).with_context(|| format!("Removing {}", path.display()))?;
    println!("Removed {}", path.display());
    Ok(())
}

pub fn remove_if_present(paths: &Paths, id: &str) -> Result<()> {
    let path = entry_path(paths, id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Removing {}", path.display())),
    }
}

fn value(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

use crate::config::{Paths, atomic_write};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn entry_path(paths: &Paths, id: &str) -> Result<PathBuf> {
    paths.game_path(id)?;
    Ok(paths
        .data
        .with_file_name("applications")
        .join(format!("drabi-{id}.desktop")))
}

pub fn create(paths: &Paths, id: &str, name: Option<&str>, icon: Option<&Path>) -> Result<()> {
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
    if let Some(icon) = icon.or(file.desktop.icon.as_deref()) {
        let icon = paths.absolute(icon)?;
        ensure!(icon.is_file(), "Icon not found at {}", icon.display());
        body.push_str(&format!(
            "Icon={}\n",
            value(icon.to_str().context("Icon path must use UTF-8")?)
        ));
    }
    let path = entry_path(paths, id)?;
    atomic_write(&path, &body, true)?;
    println!("Saved {}", path.display());
    Ok(())
}

pub fn remove(paths: &Paths, id: &str) -> Result<()> {
    let path = entry_path(paths, id)?;
    fs::remove_file(&path).with_context(|| format!("Removing {}", path.display()))?;
    println!("Removed {}", path.display());
    Ok(())
}

fn value(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace(' ', "\\s")
}

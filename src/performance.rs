use crate::runner::executable;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Performance {
    pub gamemode: bool,
    pub mangohud: bool,
}

impl Performance {
    pub fn is_empty(&self) -> bool {
        !self.gamemode && !self.mangohud
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Gamescope {
    pub enabled: bool,
    pub fullscreen: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub render_width: Option<u32>,
    pub render_height: Option<u32>,
    pub refresh_rate: Option<u32>,
    pub arguments: Vec<String>,
}

impl Gamescope {
    pub fn is_empty(&self) -> bool {
        !self.enabled
            && !self.fullscreen
            && self.width.is_none()
            && self.height.is_none()
            && self.render_width.is_none()
            && self.render_height.is_none()
            && self.refresh_rate.is_none()
            && self.arguments.is_empty()
    }
}

pub fn find_tool(tool: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter_map(|dir| std::path::absolute(dir.join(tool)).ok())
        .find(|path| executable(path))
}

fn require(tool: &str) -> Result<OsString> {
    Ok(find_tool(tool)
        .with_context(|| format!("{tool} is enabled but missing from PATH"))?
        .into_os_string())
}

pub fn wrappers(settings: &Performance, gamescope: &Gamescope) -> Result<Vec<OsString>> {
    let mut argv = Vec::new();
    if settings.gamemode {
        argv.push(require("gamemoderun")?);
    }
    if gamescope.enabled {
        argv.push(require("gamescope")?);
        for (flag, value) in [
            ("-W", gamescope.width),
            ("-H", gamescope.height),
            ("-w", gamescope.render_width),
            ("-h", gamescope.render_height),
            ("-r", gamescope.refresh_rate),
        ] {
            if let Some(value) = value {
                ensure!(value > 0, "{flag} must be positive");
                argv.push(flag.into());
                argv.push(value.to_string().into());
            }
        }
        if gamescope.fullscreen {
            argv.push("-f".into());
        }
        if settings.mangohud {
            require("mangoapp")?;
            argv.push("--mangoapp".into());
        }
        ensure!(
            !gamescope.arguments.iter().any(|arg| arg == "--"),
            "Gamescope arguments must omit the -- separator"
        );
        argv.extend(
            gamescope
                .arguments
                .iter()
                .map(OsStr::new)
                .map(OsStr::to_owned),
        );
        argv.push("--".into());
    } else if settings.mangohud {
        argv.push(require("mangohud")?);
    }
    Ok(argv)
}

use anyhow::{Context, Result, anyhow, ensure};
use std::{collections::BTreeSet, fs, path::Path, process::Command};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisplayId {
    Backlight(String),
    Ddc(u32),
}

#[derive(Clone)]
pub struct Display {
    pub id: DisplayId,
    pub name: String,
    pub level: Result<Level, String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub percent: u16,
    max: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Audio {
    pub name: String,
    pub volume: u16,
    pub muted: bool,
}

const SINK: &str = "@DEFAULT_AUDIO_SINK@";

fn command(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("timeout")
        .args(["--kill-after=1s", "6s", program])
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .context("Could not start timeout (coreutils)")?;
    let error = String::from_utf8_lossy(&output.stderr);
    let diagnostic = error
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    match output.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        Some(124 | 137) => Err(anyhow!("{program} timed out")),
        Some(127) => Err(anyhow!("{program} is unavailable")),
        _ if !diagnostic.is_empty() => Err(anyhow!("{program}: {diagnostic}")),
        _ => Err(anyhow!("{program} failed ({})", output.status)),
    }
}

fn level(current: u32, max: u32) -> Result<Level> {
    ensure!(max > 0, "Invalid maximum brightness");
    let percent = (u64::from(current.min(max)) * 100 + u64::from(max) / 2) / u64::from(max);
    Ok(Level {
        percent: percent as u16,
        max,
    })
}

fn parse_backlight(output: &str, name: &str) -> Result<Level> {
    for line in output.lines() {
        let fields: Vec<_> = line.rsplitn(5, ',').collect();
        if let [max, _, current, "backlight", device] = fields.as_slice()
            && *device == name
        {
            return level(current.parse()?, max.parse()?);
        }
    }
    Err(anyhow!("No brightness reading for {name}"))
}

fn bus_in_path(path: &Path) -> Option<u32> {
    path.components().rev().find_map(|part| {
        let part = part.as_os_str().to_str()?;
        part.strip_prefix("i2c-")
            .or_else(|| part.strip_suffix("-0037"))?
            .parse()
            .ok()
    })
}

// A bound DDC/CI client owns address 0x37. Fail closed when a ddcci backlight
// exists whose adapter cannot be identified.
fn ddc_buses(sys: &Path, backlights: &[String]) -> Option<BTreeSet<u32>> {
    let mut owned = BTreeSet::new();
    for name in backlights {
        let path = fs::canonicalize(sys.join("class/backlight").join(name)).ok();
        let ddcci = name.to_ascii_lowercase().contains("ddcci")
            || path
                .as_ref()
                .is_some_and(|path| path.to_string_lossy().contains("ddcci"));
        if ddcci {
            owned.insert(bus_in_path(path.as_deref()?)?);
        }
    }
    Some(owned)
}

fn external_buses(sys: &Path, backlights: &[String]) -> Vec<(u32, String)> {
    let Some(mut seen) = ddc_buses(sys, backlights) else {
        return Vec::new();
    };
    let Ok(connectors) = fs::read_dir(sys.join("class/drm")) else {
        return Vec::new();
    };
    let mut displays = Vec::new();
    for connector in connectors.flatten() {
        let name = connector.file_name().to_string_lossy().into_owned();
        let Some((_, name)) = name.split_once('-') else {
            continue;
        };
        if ["eDP-", "LVDS-", "DSI-"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
            || !fs::read_to_string(connector.path().join("status"))
                .is_ok_and(|status| status.trim() == "connected")
        {
            continue;
        }
        if let Ok(path) = fs::canonicalize(connector.path().join("ddc"))
            && let Some(bus) = bus_in_path(&path)
            && !bound(sys, bus)
            && seen.insert(bus)
        {
            displays.push((bus, name.to_owned()));
        }
    }
    displays.sort_by(|a, b| a.1.cmp(&b.1));
    displays
}

fn bound(sys: &Path, bus: u32) -> bool {
    sys.join(format!("bus/i2c/devices/{bus}-0037/driver"))
        .exists()
}

fn backlights(sys: &Path) -> Vec<String> {
    fs::read_dir(sys.join("class/backlight"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

pub fn displays() -> Vec<Display> {
    let sys = Path::new("/sys");
    let mut names = backlights(sys);
    names.sort();
    let mut displays = Vec::new();
    if !names.is_empty() {
        let output = command(
            "brightnessctl",
            &["--class=backlight", "--machine-readable", "--list"],
        );
        for name in &names {
            let value = output
                .as_ref()
                .map_err(ToString::to_string)
                .and_then(|output| {
                    parse_backlight(output, name).map_err(|error| error.to_string())
                });
            displays.push(Display {
                id: DisplayId::Backlight(name.clone()),
                name: name.clone(),
                level: value,
            });
        }
    }

    for (bus, name) in external_buses(sys, &names) {
        let reading = ddc_level(bus).map_err(|error| error.to_string());
        displays.push(Display {
            id: DisplayId::Ddc(bus),
            name,
            level: reading,
        });
    }
    displays
}

fn ddc_level(bus: u32) -> Result<Level> {
    let output = command(
        "ddcutil",
        &["--bus", &bus.to_string(), "--terse", "getvcp", "10"],
    )?;
    parse_ddc(&output)
}

fn parse_ddc(output: &str) -> Result<Level> {
    for line in output.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        if let ["VCP", "10", "C", current, max] = parts.as_slice() {
            return level(current.parse()?, max.parse()?);
        }
    }
    Err(anyhow!("Monitor did not report DDC/CI brightness"))
}

pub fn brightness(id: &DisplayId, max: u32, percent: u16) -> Result<Level> {
    match id {
        DisplayId::Backlight(name) => {
            let device = format!("--device={name}");
            let args = ["--class=backlight", &device, "--machine-readable"];
            command(
                "brightnessctl",
                &[args[0], args[1], args[2], "set", &format!("{percent}%")],
            )?;
            parse_backlight(
                &command("brightnessctl", &[args[0], args[1], args[2], "info"])?,
                name,
            )
        }
        DisplayId::Ddc(bus) => {
            let sys = Path::new("/sys");
            let names = backlights(sys);
            ensure!(
                external_buses(sys, &names)
                    .iter()
                    .any(|(available, _)| available == bus),
                "Monitor disconnected or now controlled by a kernel driver. Refresh devices"
            );
            let raw = (u64::from(max) * u64::from(percent) + 50) / 100;
            command(
                "ddcutil",
                &[
                    "--bus",
                    &bus.to_string(),
                    "--noverify",
                    "setvcp",
                    "10",
                    &raw.to_string(),
                ],
            )?;
            ddc_level(*bus)
        }
    }
}

pub fn audio() -> Result<Audio> {
    let reading = command("wpctl", &["get-volume", SINK])?;
    let (volume, muted) = parse_volume(&reading)?;
    let properties = command("wpctl", &["inspect", SINK])?;
    let name = ["node.nick", "node.description", "node.name"]
        .iter()
        .find_map(|key| {
            properties.lines().find_map(|line| {
                let (property, value) = line
                    .trim()
                    .trim_start_matches('*')
                    .trim()
                    .split_once(" = ")?;
                (property == *key).then(|| value.trim_matches('"').to_owned())
            })
        })
        .unwrap_or_else(|| "Default output".into());
    Ok(Audio {
        name,
        volume,
        muted,
    })
}

fn parse_volume(output: &str) -> Result<(u16, bool)> {
    let value: f64 = output
        .split_whitespace()
        .nth(1)
        .context("Missing output volume")?
        .parse()?;
    ensure!(value.is_finite() && value >= 0., "Invalid output volume");
    Ok(((value * 100.).round() as u16, output.contains("[MUTED]")))
}

pub fn volume(percent: u16) -> Result<Audio> {
    command("wpctl", &["set-volume", SINK, &format!("{percent}%")])?;
    audio()
}

pub fn mute(muted: bool) -> Result<Audio> {
    command("wpctl", &["set-mute", SINK, if muted { "1" } else { "0" }])?;
    audio()
}

impl Display {
    pub fn max(&self) -> Option<u32> {
        self.level.as_ref().ok().map(|level| level.max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn parses_reported_brightness_and_audio_levels() {
        assert_eq!(
            parse_backlight(
                "intel_backlight,backlight,38400,40%,96000",
                "intel_backlight"
            )
            .unwrap()
            .percent,
            40
        );
        assert_eq!(
            parse_ddc("VCP 10 C 50 200").unwrap(),
            Level {
                percent: 25,
                max: 200
            }
        );
        assert!(parse_ddc("VCP 10 C 50 0").is_err());
        assert_eq!(parse_volume("Volume: 0.42 [MUTED]").unwrap(), (42, true));
        assert!(parse_volume("Volume: NaN").is_err());
    }

    #[test]
    fn discovers_connected_external_buses_and_skips_kernel_owned_ddcci() {
        let dir = tempfile::tempdir().unwrap();
        let sys = dir.path();
        for (connector, bus, status) in [
            ("card1-HDMI-A-1", 9, "connected"),
            ("card1-DP-1", 12, "connected"),
            ("card1-DP-2", 14, "disconnected"),
            ("card1-eDP-1", 15, "connected"),
        ] {
            let path = sys.join("class/drm").join(connector);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("status"), status).unwrap();
            let adapter = sys.join(format!("devices/i2c-{bus}"));
            fs::create_dir_all(&adapter).unwrap();
            symlink(&adapter, path.join("ddc")).unwrap();
        }
        assert_eq!(
            external_buses(sys, &[]),
            [(12, "DP-1".into()), (9, "HDMI-A-1".into())]
        );

        let backlights = sys.join("class/backlight");
        let ddcci = sys.join("devices/i2c-9/9-0037/ddcci9");
        fs::create_dir_all(&backlights).unwrap();
        fs::create_dir_all(&ddcci).unwrap();
        symlink(ddcci, backlights.join("ddcci9")).unwrap();
        assert_eq!(
            external_buses(sys, &["ddcci9".into()]),
            [(12, "DP-1".into())]
        );

        fs::create_dir_all(sys.join("bus/i2c/devices/12-0037/driver")).unwrap();
        assert!(external_buses(sys, &["ddcci9".into()]).is_empty());
    }
}

use crate::station_protocol::SystemAction;
use anyhow::{Context, Result, ensure};
use std::process::Command;

pub fn require_unprivileged() -> Result<()> {
    ensure!(
        unsafe { libc::getuid() != 0 && libc::geteuid() != 0 },
        "Run drabi station --system as your regular user"
    );
    Ok(())
}

pub fn perform(action: SystemAction) -> Result<()> {
    let method = match action {
        // Gamescope closes when its primary client exits. A standalone login
        // session should use Gamescope itself as the session command.
        SystemAction::Logout => return Ok(()),
        SystemAction::Restart => "Reboot",
        SystemAction::Shutdown => "PowerOff",
    };
    let status = Command::new("/usr/bin/busctl")
        .args([
            "--system",
            "--allow-interactive-authorization=yes",
            "--timeout=120",
            "call",
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            method,
            "b",
            "true",
        ])
        .status()
        .with_context(|| {
            format!("Cannot request {method} through logind. Check that busctl is installed")
        })?;
    ensure!(
        status.success(),
        "logind {method} request failed ({status})"
    );
    Ok(())
}

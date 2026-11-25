use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Sandbox(tempfile::TempDir);

impl Sandbox {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.path().join(relative)
    }

    fn drabi(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_drabi"))
            .env("HOME", self.0.path())
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("UMU_NO_PROTON", "1")
            .args(args)
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> Output {
        let output = self.drabi(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

fn executable(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn runs_registered_game_and_installer_with_explicit_prefix() {
    let sandbox = Sandbox::new();
    let proton = sandbox.path("External Runner");
    executable(&proton.join("proton"), "#!/bin/sh\nexit 0\n");
    sandbox.ok(&["proton", "register", "ge", proton.to_str().unwrap()]);

    let exe = sandbox.path("Game Setup.exe");
    fs::write(&exe, "").unwrap();
    let prefix = sandbox.path("Game Prefix");
    let prefix_arg = prefix.to_str().unwrap();
    let exe_arg = exe.to_str().unwrap();

    sandbox.ok(&[
        "game",
        "add",
        "--id",
        "demo",
        "--name",
        "Demo Game",
        "--exe",
        exe_arg,
        "--prefix",
        prefix_arg,
    ]);
    assert!(!prefix.exists());
    let output = sandbox.drabi(&["run", "demo", "--dry-run"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("prefix create"));
    assert!(!sandbox.path("data").exists());

    let umu_root = sandbox.path("data/drabi/umu");
    executable(
        &umu_root.join("active/umu-run"),
        r#"#!/bin/sh
set -eu
test -z "${UMU_NO_PROTON:-}"
if [ -z "$1" ]; then
    mkdir -p "$WINEPREFIX/drive_c"
    touch "$WINEPREFIX/system.reg"
    exit 1
fi
printf '%s\n' "$PWD" "$PROTONPATH" "$WINEPREFIX" "$GAMEID" "$STORE" "$EXAMPLE" "$#"
printf '<%s>\n' "$@"
exit 23
"#,
    );
    fs::write(umu_root.join("last-check"), "").unwrap();

    let output = sandbox.drabi(&["run", "demo"]);
    assert!(!output.status.success());
    assert!(!prefix.exists());

    sandbox.ok(&["prefix", "create", prefix_arg]);
    sandbox.ok(&["prefix", "inspect", prefix_arg]);
    sandbox.ok(&["run", "demo", "--dry-run"]);

    for target in ["demo", exe_arg] {
        let output = sandbox.drabi(&[
            "run",
            target,
            "--prefix",
            prefix_arg,
            "--game-id",
            "umu-demo",
            "--store",
            "gog",
            "--arg=/S",
            "--arg=two words",
            "--env",
            "EXAMPLE=with=equals",
        ]);
        assert_eq!(output.status.code(), Some(23));
        let lines = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            lines,
            format!(
                "{}\n{}\n{prefix_arg}\numu-demo\ngog\nwith=equals\n3\n<{exe_arg}>\n</S>\n<two words>\n",
                sandbox.0.path().display(),
                proton.display()
            )
        );
    }

    sandbox.ok(&["game", "remove", "demo"]);
    sandbox.ok(&["proton", "unregister", "ge"]);
    assert!(!sandbox.path("config/drabi/games/demo.toml").exists());
    assert!(exe.is_file());
    assert!(prefix.join("system.reg").is_file());
    assert!(proton.join("proton").is_file());
}

#[test]
fn invalid_edits_preserve_the_registration() {
    let sandbox = Sandbox::new();
    let exe = sandbox.path("Game.exe");
    let editor = sandbox.path("editor");
    let registration = sandbox.path("config/drabi/games/demo.toml");
    fs::write(&exe, "").unwrap();
    sandbox.ok(&[
        "game",
        "add",
        "--id",
        "demo",
        "--name",
        "Demo Game",
        "--exe",
        exe.to_str().unwrap(),
    ]);
    let original = fs::read_to_string(&registration).unwrap();
    executable(&editor, "#!/bin/sh\nprintf 'broken = [' > \"$1\"\n");
    let output = Command::new(env!("CARGO_BIN_EXE_drabi"))
        .env("XDG_CONFIG_HOME", sandbox.path("config"))
        .env("VISUAL", editor)
        .args(["game", "edit", "demo"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Edited copy is available"));
    assert_eq!(fs::read_to_string(registration).unwrap(), original);
    let listing = sandbox.ok(&["game", "list"]);
    assert_eq!(listing.stdout, b"demo\tDemo Game\n");
}

#[test]
fn automatic_update_failure_reuses_the_installed_umu() {
    let sandbox = Sandbox::new();
    let proton = sandbox.path("Runner");
    let exe = sandbox.path("Game.exe");
    let prefix = sandbox.path("Prefix");
    let tools = sandbox.path("bin");
    let umu = sandbox.path("data/drabi/umu");

    executable(&proton.join("proton"), "#!/bin/sh\nexit 0\n");
    executable(&tools.join("curl"), "#!/bin/sh\nexit 7\n");
    executable(&umu.join("active/umu-run"), "#!/bin/sh\nexit 0\n");
    fs::write(&exe, "").unwrap();
    fs::create_dir_all(prefix.join("drive_c")).unwrap();
    fs::write(prefix.join("system.reg"), "").unwrap();
    sandbox.ok(&["proton", "register", "ge", proton.to_str().unwrap()]);

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_drabi"))
            .env("XDG_CONFIG_HOME", sandbox.path("config"))
            .env("XDG_DATA_HOME", sandbox.path("data"))
            .env("PATH", &tools)
            .args([
                "run",
                exe.to_str().unwrap(),
                "--prefix",
                prefix.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    let first = run();
    assert!(first.status.success());
    assert!(String::from_utf8_lossy(&first.stderr).contains("using installed version"));
    assert!(umu.join("last-check").is_file());

    let second = run();
    assert!(second.status.success());
    assert!(second.stderr.is_empty());
}

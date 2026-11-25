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
    let preview = sandbox.ok(&["run", "demo", "--dry-run", "--env", "EXAMPLE=with=equals"]);
    let preview = String::from_utf8(preview.stdout).unwrap();
    assert!(preview.contains("Command argv:"));
    assert!(preview.contains("Additional environment keys: [\"EXAMPLE\"]"));
    assert!(!preview.contains("with=equals"));

    let preview = sandbox.ok(&[
        "run",
        "demo",
        "--dry-run",
        "--show-env",
        "--env",
        "EXAMPLE=with=equals",
    ]);
    let preview = String::from_utf8(preview.stdout).unwrap();
    assert!(preview.contains("EXAMPLE=\"with=equals\""));
    assert!(preview.contains("\"UMU_NO_PROTON\""));

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

#[test]
fn creates_updates_and_removes_desktop_entries() {
    let sandbox = Sandbox::new();
    let exe = sandbox.path("Game.exe");
    let icon = sandbox.path("Game icon.png");
    fs::write(&exe, "").unwrap();
    fs::write(&icon, "").unwrap();
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
    let entry = sandbox.path("data/applications/drabi-demo.desktop");
    let preview = sandbox.ok(&["desktop", "create", "demo", "--dry-run"]);
    assert!(String::from_utf8_lossy(&preview.stdout).contains("Icon: desktop default"));
    let preview = sandbox.ok(&[
        "desktop",
        "create",
        "demo",
        "--icon",
        icon.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(String::from_utf8_lossy(&preview.stdout).contains("Icon="));
    assert!(!sandbox.path("data").exists());
    sandbox.ok(&[
        "desktop",
        "create",
        "demo",
        "--icon",
        icon.to_str().unwrap(),
    ]);
    let initial = fs::read_to_string(&entry).unwrap();
    assert!(initial.contains("Name=Demo Game\n"));
    assert!(initial.contains(" run demo\n"));
    assert!(initial.contains("Icon="));
    sandbox.ok(&["desktop", "create", "demo", "--name", "Updated"]);
    assert!(
        fs::read_to_string(&entry)
            .unwrap()
            .contains("Name=Updated\n")
    );
    sandbox.ok(&["desktop", "remove", "demo"]);
    assert!(!entry.exists());
    assert!(sandbox.path("config/drabi/games/demo.toml").exists());
    assert!(exe.is_file());
}

#[test]
fn saves_playtime_only_when_enabled_and_runs_a_post_exit_command() {
    let sandbox = Sandbox::new();
    let runner = sandbox.path("runner");
    let prefix = sandbox.path("prefix");
    let exe = sandbox.path("Game.exe");
    let umu = sandbox.path("data/drabi/umu");
    executable(&runner.join("proton"), "#!/bin/sh\nexit 0\n");
    executable(&umu.join("active/umu-run"), "#!/bin/sh\nexit 17\n");
    fs::write(umu.join("last-check"), "").unwrap();
    fs::write(&exe, "").unwrap();
    fs::create_dir_all(prefix.join("drive_c")).unwrap();
    fs::write(prefix.join("system.reg"), "").unwrap();
    sandbox.ok(&["proton", "register", "runner", runner.to_str().unwrap()]);
    sandbox.ok(&[
        "game",
        "add",
        "--id",
        "demo",
        "--name",
        "Demo",
        "--exe",
        exe.to_str().unwrap(),
        "--prefix",
        prefix.to_str().unwrap(),
    ]);
    assert_eq!(sandbox.drabi(&["run", "demo"]).status.code(), Some(17));
    let record = sandbox.path("data/drabi/playtime/demo");
    assert!(!record.exists());

    let exit_hook = sandbox.path("exit hook");
    executable(
        &exit_hook,
        "#!/bin/sh\nprintf '%s' \"$DRABI_EXIT_CODE\" > 'result'\n",
    );
    let game = sandbox.path("config/drabi/games/demo.toml");
    let mut body = fs::read_to_string(&game).unwrap();
    body.push_str(&format!(
        "\n[lifecycle]\ntrack_playtime = true\npost_exit = [{:?}]\n",
        exit_hook.to_str().unwrap()
    ));
    fs::write(game, body).unwrap();

    let output = sandbox.drabi(&["run", "demo", "--detached"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("require a managed launch"));
    assert_eq!(sandbox.drabi(&["run", "demo"]).status.code(), Some(17));
    assert!(record.is_file());
    assert_eq!(fs::read_to_string(sandbox.path("result")).unwrap(), "17");
}

#[test]
fn manages_verified_proton_releases_and_preserves_other_runners() {
    for (source, repo, tag, suffix) in [
        (
            "ge",
            "GloriousEggroll/proton-ge-custom",
            "GE-Proton99-1",
            "gz",
        ),
        ("dw", "dawn-winery/dwproton-mirror", "dwproton-99.0-1", "xz"),
        (
            "umu",
            "Open-Wine-Components/umu-proton",
            "UMU-Proton-99.0-1",
            "gz",
        ),
    ] {
        check_runner_release(source, repo, tag, suffix);
    }
}

fn check_runner_release(source: &str, repo: &str, tag: &str, suffix: &str) {
    use sha2::{Digest, Sha256};
    use std::io::Write;

    let sandbox = Sandbox::new();
    let asset = format!("{tag}-{}.tar.{suffix}", std::env::consts::ARCH);
    let script = b"#!/bin/sh\nexit 0\n";
    let mut header = tar::Header::new_gnu();
    header.set_size(script.len() as u64);
    header.set_mode(0o755);
    let mut archive = tar::Builder::new(Vec::new());
    archive
        .append_data(&mut header, format!("{tag}/proton"), &script[..])
        .unwrap();
    let archive = archive.into_inner().unwrap();
    let compressed = if suffix == "xz" {
        let mut encoder = xz2::write::XzEncoder::new(Vec::new(), 1);
        encoder.write_all(&archive).unwrap();
        encoder.finish().unwrap()
    } else {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&archive).unwrap();
        encoder.finish().unwrap()
    };
    fs::write(sandbox.path("asset"), &compressed).unwrap();

    let release = serde_json::json!({
        "tag_name": tag, "prerelease": false, "assets": [{
            "name": asset,
            "browser_download_url": format!("https://github.com/{repo}/releases/download/{tag}/{asset}"),
            "digest": format!("sha256:{:x}", Sha256::digest(&compressed)),
        }]
    });
    fs::write(sandbox.path("release"), release.to_string()).unwrap();
    let legacy_tag = "ULWGL-Proton-8.0-5-3";
    let legacy = serde_json::json!({
        "tag_name": legacy_tag, "prerelease": false, "assets": [{
            "name": format!("{legacy_tag}-{}.tar.gz", std::env::consts::ARCH),
            "browser_download_url": "unused",
            "digest": "sha256:unused",
        }]
    });
    fs::write(sandbox.path("releases"), format!("[{release},{legacy}]")).unwrap();
    let tools = sandbox.path("tools");
    executable(
        &tools.join("curl"),
        r#"#!/bin/sh
set -eu
out=''
for arg do
    if [ "${previous:-}" = --output ]; then out=$arg; fi
    previous=$arg
done
case $previous in
    */releases\?*) /bin/cat "$HOME/releases";;
    */releases/latest|*/releases/tags/*) /bin/cat "$HOME/release";;
    */releases/download/*) /bin/cp "$HOME/asset" "$out";;
    *) exit 1;;
esac
"#,
    );
    let command = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_drabi"))
            .env("HOME", sandbox.0.path())
            .env("XDG_CONFIG_HOME", sandbox.path("config"))
            .env("XDG_DATA_HOME", sandbox.path("data"))
            .env("PATH", &tools)
            .args(args)
            .output()
            .unwrap();
        if args[1] == "remove" && args[2] != tag {
            assert!(!output.status.success());
        } else {
            assert!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8(output.stdout).unwrap()
    };

    let available = command(&["proton", "list", "--available", "--source", source]);
    assert!(available.contains("available"));
    assert!(!available.contains(legacy_tag));
    command(&["proton", "install", tag]);
    assert!(command(&["proton", "search", "--source", source]).contains("installed"));
    assert!(command(&["proton", "list"]).contains("managed"));
    command(&["proton", "update", tag]);
    let unmanaged = sandbox.path("data/drabi/runners/proton/GE-Proton99-2/proton");
    executable(&unmanaged, "#!/bin/sh\nexit 0\n");
    command(&["proton", "remove", "GE-Proton99-2"]);
    assert!(unmanaged.is_file());
    command(&["proton", "remove", tag]);
    assert!(
        !sandbox
            .path(&format!("data/drabi/runners/proton/{tag}"))
            .exists()
    );
}

#[test]
fn wraps_a_detached_launch_and_keeps_its_log() {
    let sandbox = Sandbox::new();
    let proton = sandbox.path("runner");
    let exe = sandbox.path("Game.exe");
    let prefix = sandbox.path("prefix");
    let tools = sandbox.path("tools");
    let umu = sandbox.path("data/drabi/umu");
    executable(&proton.join("proton"), "#!/bin/sh\nexit 0\n");
    executable(
        &umu.join("active/umu-run"),
        "#!/bin/sh\nprintf 'launched %s\\n' \"$1\"\n",
    );
    executable(
        &tools.join("gamemoderun"),
        "#!/bin/sh\nprintf 'gamemode\\n'\nexec \"$@\"\n",
    );
    executable(&tools.join("mangoapp"), "#!/bin/sh\nexit 0\n");
    executable(
        &tools.join("gamescope"),
        "#!/bin/sh\nprintf 'gamescope %s\\n' \"$1\"\n[ \"$1\" = --mangoapp ] && [ \"$2\" = -- ] || exit 1\nshift 2\nexec \"$@\"\n",
    );
    fs::write(umu.join("last-check"), "").unwrap();
    fs::write(&exe, "").unwrap();
    fs::create_dir_all(prefix.join("drive_c")).unwrap();
    fs::write(prefix.join("system.reg"), "").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_drabi"))
        .env("XDG_CONFIG_HOME", sandbox.path("config"))
        .env("XDG_DATA_HOME", sandbox.path("data"))
        .env("PATH", &tools)
        .args([
            "run",
            exe.to_str().unwrap(),
            "--prefix",
            prefix.to_str().unwrap(),
            "--runner",
            proton.to_str().unwrap(),
            "--detached",
            "--gamescope",
            "--mangohud",
            "--gamemode",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let log = stdout.split_once("Log at ").unwrap().1.trim();
    let expected = format!(
        "gamemode\ngamescope --mangoapp\nlaunched {}\n",
        exe.display()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let contents = fs::read_to_string(log).unwrap();
        if contents == expected {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Unexpected log: {contents}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn lists_steam_tools_and_opens_their_install_request() {
    let sandbox = Sandbox::new();
    let tools = sandbox.path("tools");
    executable(
        &tools.join("xdg-open"),
        "#!/bin/sh\nprintf '%s\\n' \"$1\"\n",
    );
    let listing = sandbox.ok(&["proton", "list", "--available", "--source", "valve"]);
    assert!(String::from_utf8_lossy(&listing.stdout).contains("proton-experimental\tavailable"));
    let output = Command::new(env!("CARGO_BIN_EXE_drabi"))
        .env("HOME", sandbox.0.path())
        .env("XDG_DATA_HOME", sandbox.path("data"))
        .env("PATH", &tools)
        .args(["proton", "install", "proton-experimental"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("steam://install/1493710"));

    executable(
        &sandbox.path("data/Steam/steamapps/common/Proton - Experimental/proton"),
        "#!/bin/sh\nexit 0\n",
    );
    let local = sandbox.ok(&["proton", "list"]);
    assert!(String::from_utf8_lossy(&local.stdout).contains("proton-experimental\tsteam"));
}

#[test]
fn rejects_foreign_prefix_symlinks_and_runs_winetricks_verbs_separately() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new();
    let wine = sandbox.path("compat/pfx");
    fs::create_dir_all(wine.join("drive_c")).unwrap();
    fs::write(wine.join("system.reg"), "").unwrap();
    let foreign = sandbox.path("foreign");
    fs::create_dir(&foreign).unwrap();
    symlink(&wine, foreign.join("pfx")).unwrap();
    let runner = sandbox.path("runner");
    executable(&runner.join("proton"), "#!/bin/sh\nexit 0\n");
    let umu = sandbox.path("data/drabi/umu");
    executable(
        &umu.join("active/umu-run"),
        "#!/bin/sh\n[ \"$#\" = 2 ] || exit 99\nprintf '%s:%s\\n' \"$WINEPREFIX\" \"$2\"\n[ \"$2\" != first ]\n",
    );
    fs::write(umu.join("last-check"), "").unwrap();

    let compat = sandbox.path("compat");
    sandbox.ok(&["prefix", "inspect", compat.to_str().unwrap()]);
    symlink(".", wine.join("pfx")).unwrap();
    sandbox.ok(&["prefix", "inspect", wine.to_str().unwrap()]);
    let output = sandbox.drabi(&["prefix", "inspect", foreign.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("UMU would replace it"));
    let output = sandbox.drabi(&[
        "prefix",
        "winetricks",
        foreign.to_str().unwrap(),
        "second",
        "--runner",
        runner.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read_link(foreign.join("pfx")).unwrap(), wine);

    let output = sandbox.drabi(&[
        "prefix",
        "winetricks",
        compat.to_str().unwrap(),
        "first",
        "second",
        "--runner",
        runner.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{}:first\n{}:second\n", compat.display(), compat.display())
    );
}

#[test]
fn blocks_removal_during_managed_and_detached_launches() {
    for detached in [false, true] {
        let sandbox = Sandbox::new();
        let proton = sandbox.path("data/drabi/runners/proton/GE-Proton99-1");
        executable(&proton.join("proton"), "#!/bin/sh\nexit 0\n");
        fs::write(proton.join(".drabi-managed"), "").unwrap();
        let exe = sandbox.path("Game.exe");
        fs::write(&exe, "").unwrap();
        let prefix = sandbox.path("prefix");
        fs::create_dir_all(prefix.join("drive_c")).unwrap();
        fs::write(prefix.join("system.reg"), "").unwrap();
        let umu = sandbox.path("data/drabi/umu");
        executable(
            &umu.join("active/umu-run"),
            "#!/bin/sh\ntouch \"$HOME/started\"\nfor attempt in $(seq 1 100); do\n  [ -e \"$HOME/finish\" ] && exit 0\n  sleep .05\ndone\nexit 98\n",
        );
        fs::write(umu.join("last-check"), "").unwrap();
        let mut args = vec![
            "run",
            exe.to_str().unwrap(),
            "--prefix",
            prefix.to_str().unwrap(),
            "--runner",
            "GE-Proton99-1",
        ];
        if detached {
            args.push("--detached");
        }
        let sandbox = &sandbox;
        std::thread::scope(|scope| {
            let child = scope.spawn(|| sandbox.drabi(&args));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
            while !sandbox.path("started").exists() {
                assert!(std::time::Instant::now() < deadline, "Launch did not start");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let output = sandbox.drabi(&["proton", "remove", "GE-Proton99-1"]);
            fs::write(sandbox.path("finish"), "").unwrap();
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("currently in use"));
            assert!(proton.join("proton").is_file());
            assert!(child.join().unwrap().status.success());
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        loop {
            let output = sandbox.drabi(&["proton", "remove", "GE-Proton99-1"]);
            if output.status.success() {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{:?}", output);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[test]
fn uses_active_umu_when_the_update_lock_cannot_be_opened() {
    let sandbox = Sandbox::new();
    let proton = sandbox.path("runner");
    executable(&proton.join("proton"), "#!/bin/sh\nexit 0\n");
    let prefix = sandbox.path("prefix");
    fs::create_dir_all(prefix.join("drive_c")).unwrap();
    fs::write(prefix.join("system.reg"), "").unwrap();
    let exe = sandbox.path("Game.exe");
    fs::write(&exe, "").unwrap();
    let umu = sandbox.path("data/drabi/umu");
    executable(&umu.join("active/umu-run"), "#!/bin/sh\nexit 0\n");
    fs::create_dir(umu.join(".install.lock")).unwrap();
    let output = sandbox.ok(&[
        "run",
        exe.to_str().unwrap(),
        "--prefix",
        prefix.to_str().unwrap(),
        "--runner",
        proton.to_str().unwrap(),
    ]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("using installed version"));
    assert!(!sandbox.drabi(&["update"]).status.success());
}

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
    let text = String::from_utf8(listing.stdout).unwrap();
    assert!(text.contains("Demo Game"));
    assert!(text.contains('│'));
    assert!(!text.contains('\t'));
    let listing = sandbox.ok(&["game", "list", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_slice(&listing.stdout).unwrap();
    assert_eq!(
        json,
        serde_json::json!([{ "id": "demo", "name": "Demo Game" }])
    );
    let shown = sandbox.ok(&["game", "show", "demo", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(json["game"]["name"], "Demo Game");
}

#[test]
fn only_explicit_update_checks_for_a_new_umu_when_one_is_active() {
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

    let invoke = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_drabi"))
            .env("XDG_CONFIG_HOME", sandbox.path("config"))
            .env("XDG_DATA_HOME", sandbox.path("data"))
            .env("PATH", &tools)
            .args(args)
            .output()
            .unwrap()
    };
    let run = || {
        invoke(&[
            "run",
            exe.to_str().unwrap(),
            "--prefix",
            prefix.to_str().unwrap(),
        ])
    };
    let first = run();
    assert!(first.status.success());
    assert!(first.stderr.is_empty());
    assert!(!umu.join(".install.lock").exists());

    let update = invoke(&["update"]);
    assert!(!update.status.success());
    assert!(
        String::from_utf8_lossy(&update.stderr).contains("Fetching UMU release metadata failed")
    );
    assert!(run().status.success());

    fs::rename(umu.join("active"), umu.join("version-fixture")).unwrap();
    executable(
        &tools.join("curl"),
        r#"#!/bin/sh
printf '%s\n' '{"tag_name":"fixture","assets":[]}'
"#,
    );
    assert!(run().status.success());
    assert_eq!(
        fs::read_link(umu.join("active")).unwrap(),
        Path::new("version-fixture")
    );
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
    let preview = sandbox.ok(&["game", "create-shortcut", "demo", "--dry-run"]);
    assert!(String::from_utf8_lossy(&preview.stdout).contains("Icon: desktop default"));
    let preview = sandbox.ok(&[
        "game",
        "create-shortcut",
        "demo",
        "--icon",
        icon.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(String::from_utf8_lossy(&preview.stdout).contains("Icon="));
    assert!(!sandbox.path("data").exists());
    sandbox.ok(&[
        "game",
        "create-shortcut",
        "demo",
        "--icon",
        icon.to_str().unwrap(),
    ]);
    let initial = fs::read_to_string(&entry).unwrap();
    assert!(initial.contains("Name=Demo Game\n"));
    assert!(initial.contains(" run demo\n"));
    assert!(initial.contains("Icon="));
    sandbox.ok(&["game", "create-shortcut", "demo", "--name", "Updated"]);
    assert!(
        fs::read_to_string(&entry)
            .unwrap()
            .contains("Name=Updated\n")
    );
    sandbox.ok(&["game", "remove-shortcut", "demo"]);
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
fn managed_hooks_run_in_order_and_clean_up_after_failures() {
    let sandbox = Sandbox::new();
    let runner = sandbox.path("runner");
    let prefix = sandbox.path("prefix");
    let exe = sandbox.path("Game.exe");
    let directory = sandbox.path("working directory");
    let umu = sandbox.path("data/drabi/umu");
    let launcher = umu.join("active/umu-run");
    let pre = sandbox.path("pre hook");
    let post = sandbox.path("post hook");
    executable(&runner.join("proton"), "#!/bin/sh\nexit 0\n");
    fs::write(&exe, "").unwrap();
    fs::create_dir_all(&directory).unwrap();
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
        "--working-dir",
        directory.to_str().unwrap(),
    ]);
    executable(
        &pre,
        r#"#!/bin/sh
printf 'pre:%s:%s:%s\n' "$1" "$GAMEID" "$MODE" >> events
if [ "$MODE" = pre-failure ]; then
    echo 'pre hook diagnostic' >&2
    exit 23
fi
"#,
    );
    executable(
        &post,
        r#"#!/bin/sh
printf 'post:%s:%s\n' "$DRABI_EXIT_CODE" "$MODE" >> events
"#,
    );
    let game = sandbox.path("config/drabi/games/demo.toml");
    let original = fs::read_to_string(&game).unwrap();
    let body = format!(
        "{original}\n[lifecycle]\ndetached = true\ntrack_playtime = true\npre_launch = [{:?}, \"literal ; argument\"]\npost_exit = [{:?}]\n",
        pre.to_str().unwrap(),
        post.to_str().unwrap()
    );
    fs::write(&game, &body).unwrap();
    let normal_launcher = r#"#!/bin/sh
printf 'game:%s\n' "$MODE" >> events
case "$MODE" in
    game-failure) exit 17 ;;
    game-signal) kill -TERM "$$" ;;
esac
"#;
    executable(&launcher, normal_launcher);

    let events = directory.join("events");
    let playtime = sandbox.path("data/drabi/playtime/demo");
    let rejected = sandbox.drabi(&["run", "demo"]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("require a managed launch"));
    sandbox.ok(&["run", "demo", "--managed", "--dry-run"]);
    assert!(!events.exists());

    for (mode, status, game_ran, cleanup_code, diagnostic) in [
        ("pre-failure", 23, false, 23, "pre hook diagnostic"),
        (
            "startup-failure",
            1,
            false,
            125,
            "Starting the managed UMU launcher",
        ),
        ("success", 0, true, 0, ""),
        ("game-failure", 17, true, 17, ""),
        ("game-signal", 143, true, 143, ""),
    ] {
        executable(
            &launcher,
            if mode == "startup-failure" {
                "#!/drabi-missing-interpreter\n"
            } else {
                normal_launcher
            },
        );
        let output = sandbox.drabi(&["run", "demo", "--managed", "--env", &format!("MODE={mode}")]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(status), "{mode}: {stderr}");
        assert!(stderr.contains(diagnostic), "{mode}: {stderr}");
        if mode == "pre-failure" {
            assert!(stderr.contains("pre_launch hook exited with exit status: 23"));
        }
        let mut expected = format!("pre:literal ; argument:umu-default:{mode}\n");
        if game_ran {
            expected.push_str(&format!("game:{mode}\n"));
        }
        expected.push_str(&format!("post:{cleanup_code}:{mode}\n"));
        assert_eq!(fs::read_to_string(&events).unwrap(), expected, "{mode}");
        assert_eq!(playtime.exists(), game_ran, "{mode}");
        fs::remove_file(&events).unwrap();
        if game_ran {
            fs::remove_file(&playtime).unwrap();
        }
    }

    for invalid in ["\"pre hook --flag\"", "[\"\"]"] {
        fs::write(
            &game,
            format!("{original}\n[lifecycle]\npre_launch = {invalid}\n"),
        )
        .unwrap();
        assert!(
            !sandbox
                .drabi(&["run", "demo", "--dry-run"])
                .status
                .success()
        );
    }
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
    let listing = sandbox.ok(&[
        "proton",
        "list",
        "--available",
        "--source",
        "valve",
        "--format",
        "json",
    ]);
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&listing.stdout).unwrap();
    assert!(
        rows.iter()
            .any(|row| row["version"] == "proton-experimental"
                && row["status"] == "available"
                && row["steam_app_id"] == 1493710)
    );
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
    let local = sandbox.ok(&["proton", "list", "--format", "json"]);
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&local.stdout).unwrap();
    assert!(
        rows.iter()
            .any(|row| row["name"] == "proton-experimental" && row["source"] == "steam")
    );
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
    assert!(output.stderr.is_empty());
    assert!(!sandbox.drabi(&["update"]).status.success());
}

#[test]
fn opens_registry_editor_and_control_panel_in_the_selected_prefix() {
    let sandbox = Sandbox::new();
    let prefix = sandbox.path("compat");
    fs::create_dir_all(prefix.join("pfx/drive_c")).unwrap();
    fs::write(prefix.join("pfx/system.reg"), "").unwrap();
    let proton = sandbox.path("runner");
    executable(&proton.join("proton"), "#!/bin/sh\nexit 0\n");
    let umu = sandbox.path("data/drabi/umu");
    executable(
        &umu.join("active/umu-run"),
        "#!/bin/sh\n[ \"$#\" = 1 ] || exit 98\nprintf '%s\\n' \"$WINEPREFIX\" \"$PROTONPATH\" \"$1\"\n",
    );

    for (command, executable) in [("regedit", "regedit.exe"), ("control", "control.exe")] {
        let output = sandbox.ok(&[
            "prefix",
            command,
            prefix.to_str().unwrap(),
            "--runner",
            proton.to_str().unwrap(),
        ]);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{}\n{}\n{executable}\n", prefix.display(), proton.display())
        );

        let missing = sandbox.path("missing");
        let output = sandbox.drabi(&[
            "prefix",
            command,
            missing.to_str().unwrap(),
            "--runner",
            proton.to_str().unwrap(),
        ]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("uninitialized"));
        assert!(!missing.exists());
    }
}

#[test]
fn resolves_global_defaults_then_uses_the_first_available_runner() {
    let sandbox = Sandbox::new();
    let exe = sandbox.path("Game.exe");
    fs::write(&exe, "").unwrap();
    let config = sandbox.path("config/drabi/config.toml");
    let first = sandbox.path("first");
    let other = sandbox.path("other");
    for runner in [&first, &other] {
        executable(&runner.join("proton"), "#!/bin/sh\nexit 0\n");
    }
    for prefix in [".wine", "global", "game-prefix", "explicit"] {
        let prefix = sandbox.path(prefix);
        fs::create_dir_all(prefix.join("drive_c")).unwrap();
        fs::write(prefix.join("system.reg"), "").unwrap();
    }
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let external = "[external_runners]\nz-last = \"~/other\"\na-first = \"~/first\"\n";
    fs::write(&config, external).unwrap();
    let output = sandbox.ok(&["run", exe.to_str().unwrap(), "--dry-run"]);
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains(&format!("WINEPREFIX={}\n", sandbox.path(".wine").display())));
    assert!(output.contains(&format!("PROTONPATH={}\n", first.display())));

    fs::write(
        &config,
        format!("default_prefix = \"~/global\"\ndefault_runner = \"z-last\"\n{external}"),
    )
    .unwrap();
    let output = sandbox.ok(&["run", exe.to_str().unwrap(), "--dry-run"]);
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains(&format!(
        "WINEPREFIX={}\n",
        sandbox.path("global").display()
    )));
    assert!(output.contains(&format!("PROTONPATH={}\n", other.display())));

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
        sandbox.path("game-prefix").to_str().unwrap(),
        "--runner",
        "a-first",
    ]);
    let output = sandbox.ok(&["run", "demo", "--dry-run"]);
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains(&format!(
        "WINEPREFIX={}\n",
        sandbox.path("game-prefix").display()
    )));
    assert!(output.contains(&format!("PROTONPATH={}\n", first.display())));

    let output = sandbox.ok(&[
        "run",
        "demo",
        "--dry-run",
        "--prefix",
        sandbox.path("explicit").to_str().unwrap(),
        "--runner",
        "z-last",
    ]);
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains(&format!(
        "WINEPREFIX={}\n",
        sandbox.path("explicit").display()
    )));
    assert!(output.contains(&format!("PROTONPATH={}\n", other.display())));
}

#[test]
fn prefix_commands_use_global_defaults_when_omitted() {
    let sandbox = Sandbox::new();
    let proton = sandbox.path("runner");
    executable(&proton.join("proton"), "#!/bin/sh\nexit 0\n");
    let prefix = sandbox.path("default");
    let config = sandbox.path("config/drabi/config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, "default_prefix = \"~/default\"\ndefault_runner = \"dw\"\n[external_runners]\ndw = \"~/runner\"\n").unwrap();
    let umu = sandbox.path("data/drabi/umu");
    executable(
        &umu.join("active/umu-run"),
        "#!/bin/sh\nif [ -z \"$1\" ]; then mkdir -p \"$WINEPREFIX/drive_c\"; touch \"$WINEPREFIX/system.reg\"; exit 1; fi\nprintf '%s\\n' \"$WINEPREFIX\" \"$PROTONPATH\" \"$@\"\n",
    );

    sandbox.ok(&["prefix", "create"]);
    let output = sandbox.ok(&["prefix", "inspect"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains(prefix.to_str().unwrap()));
    let output = sandbox.ok(&["prefix", "list"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains(prefix.to_str().unwrap()));

    for cmd in ["winecfg", "regedit", "control"] {
        let output = sandbox.ok(&["prefix", cmd]);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{}\n{}\n{cmd}.exe\n", prefix.display(), proton.display())
        );
    }
    let output = sandbox.ok(&["prefix", "winetricks", "list-installed"]);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "{}\n{}\nwinetricks\nlist-installed\n",
            prefix.display(),
            proton.display()
        )
    );

    let override_prefix = sandbox.path("override");
    sandbox.ok(&["prefix", "create", override_prefix.to_str().unwrap()]);
    let output = sandbox.ok(&[
        "prefix",
        "winetricks",
        "list-installed",
        "--prefix",
        override_prefix.to_str().unwrap(),
    ]);
    assert!(String::from_utf8_lossy(&output.stdout).starts_with(override_prefix.to_str().unwrap()));
}

fn discovery_pe(path: &Path) {
    let mut data = vec![0u8; 512];
    data[..2].copy_from_slice(b"MZ");
    data[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
    data[64..68].copy_from_slice(b"PE\0\0");
    data[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
    data[84..86].copy_from_slice(&240u16.to_le_bytes());
    data[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
    data[144..148].copy_from_slice(&4096u32.to_le_bytes());
    data[148..152].copy_from_slice(&512u32.to_le_bytes());
    fs::write(path, data).unwrap();
}

fn discovery_shortcut(path: &Path, target: &str, directory: &str, arguments: &str) {
    let mut data = vec![0u8; 76];
    data[..4].copy_from_slice(&76u32.to_le_bytes());
    data[4..20].copy_from_slice(&[
        0x01, 0x14, 0x02, 0x00, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46,
    ]);
    data[20..24].copy_from_slice(&0xb8u32.to_le_bytes());
    data[60..64].copy_from_slice(&1u32.to_le_bytes());
    for value in [target, directory, arguments] {
        let utf16: Vec<_> = value.encode_utf16().collect();
        data.extend_from_slice(&(utf16.len() as u16).to_le_bytes());
        for unit in utf16 {
            data.extend_from_slice(&unit.to_le_bytes());
        }
    }
    data.extend_from_slice(&[0; 4]);
    fs::write(path, data).unwrap();
}

#[test]
fn installs_discovers_and_uninstalls_on_the_selected_prefix() {
    let sandbox = Sandbox::new();
    executable(&sandbox.path("runner/proton"), "#!/bin/sh\nexit 0\n");
    sandbox.ok(&[
        "proton",
        "register",
        "dw",
        sandbox.path("runner").to_str().unwrap(),
    ]);
    let umu = sandbox.path("data/drabi/umu");
    let script = r#"#!/bin/sh
set -eu
if [ -z "$1" ]; then
    mkdir -p "$WINEPREFIX/drive_c"
    printf 'WINE REGISTRY Version 2\n' > "$WINEPREFIX/system.reg"
    exit 1
fi
case "$1" in
    */Setup.exe)
        mkdir -p "$WINEPREFIX/drive_c/Games/Sample" "$WINEPREFIX/drive_c/users/test/Desktop"
        for exe in Sample.exe Alternate.exe unins000.exe; do
            cp "$HOME/payload" "$WINEPREFIX/drive_c/Games/Sample/$exe"
        done
        cp "$HOME/link" "$WINEPREFIX/drive_c/users/test/Desktop/Sample Game.lnk"
        cp "$HOME/registry" "$WINEPREFIX/user.reg"
        ;;
    */unins000.exe)
        [ "$#" = 2 ] && [ "$2" = '/confirm=Sample Game' ]
        case "$(cat "$HOME/uninstall-mode")" in
            failure) exit 7 ;;
            unchanged) exit 0 ;;
        esac
        rm "$WINEPREFIX/user.reg" "$WINEPREFIX/drive_c/Games/Sample/Sample.exe"
        ;;
    */Cancelled.exe) exit 12 ;;
    */NoEvidence.exe) exit 0 ;;
    *) exit 99 ;;
esac
"#;
    executable(&umu.join("active/umu-run"), script);
    fs::write(sandbox.path("Setup.exe"), "").unwrap();
    fs::write(sandbox.path("Cancelled.exe"), "").unwrap();
    fs::write(sandbox.path("NoEvidence.exe"), "").unwrap();
    discovery_pe(&sandbox.path("payload"));
    discovery_shortcut(
        &sandbox.path("link"),
        r"C:\GAMES\SAMPLE\Sample.exe",
        r"C:\Games\Sample",
        r#"--mode "two words""#,
    );
    fs::write(
        sandbox.path("registry"),
        r#"WINE REGISTRY Version 2
[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Sample] 123
"DisplayName"="Sample Game"
"DisplayVersion"="1.2.3"
"Publisher"="Sample Studio"
"InstallLocation"="C:\\Games\\Sample"
"DisplayIcon"="C:\\Games\\Sample\\Alternate.exe,0"
"UninstallString"="\"C:\\Games\\Sample\\unins000.exe\" \"/confirm=Sample Game\""
"#,
    )
    .unwrap();

    for (file, error) in [
        ("Cancelled.exe", "status 12"),
        ("NoEvidence.exe", "No unique game executable"),
    ] {
        let result = sandbox.drabi(&[
            "game",
            "install",
            sandbox.path(file).to_str().unwrap(),
            "--isolated-prefix",
        ]);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains(error));
    }
    sandbox.ok(&[
        "game",
        "install",
        sandbox.path("Setup.exe").to_str().unwrap(),
        "--isolated-prefix",
    ]);
    let id = "sample-game";
    let file = sandbox.ok(&["game", "show", id, "--format", "json"]);
    let game: serde_json::Value = serde_json::from_slice(&file.stdout).unwrap();
    let prefix = PathBuf::from(game["prefix"]["path"].as_str().unwrap());
    assert!(
        game["game"]["exe"]
            .as_str()
            .unwrap()
            .ends_with("/Sample.exe")
    );
    assert!(
        game["game"]["working_dir"]
            .as_str()
            .unwrap()
            .ends_with("/Games/Sample")
    );
    assert_eq!(
        game["game"]["arguments"],
        serde_json::json!(["--mode", "two words"])
    );
    assert_eq!(game["installation"]["publisher"], "Sample Studio");
    assert_eq!(game["installation"]["version"], "1.2.3");
    sandbox.ok(&["game", "create-shortcut", id]);
    let entry = sandbox.path("data/applications/drabi-sample-game.desktop");
    let config = sandbox.path("config/drabi/games/sample-game.toml");
    assert!(entry.is_file());

    let confirmed_uninstall = |answer: &str, purge: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_drabi"));
        command
            .env("HOME", sandbox.0.path())
            .env("XDG_CONFIG_HOME", sandbox.path("config"))
            .env("XDG_DATA_HOME", sandbox.path("data"))
            .args(["game", "uninstall", id]);
        if purge {
            command.arg("--purge");
        }
        let input = sandbox.path("uninstall-answer");
        fs::write(&input, answer).unwrap();
        command
            .stdin(fs::File::open(&input).unwrap())
            .output()
            .unwrap()
    };
    assert!(!confirmed_uninstall("no\n", false).status.success());
    for mode in ["failure", "unchanged"] {
        fs::write(sandbox.path("uninstall-mode"), mode).unwrap();
        assert!(!confirmed_uninstall("yes\n", false).status.success());
        assert!(entry.is_file() && config.is_file() && prefix.is_dir());
    }

    let exe = game["game"]["exe"].as_str().unwrap();
    sandbox.ok(&[
        "game",
        "add",
        "--id",
        "shared",
        "--name",
        "Shared",
        "--exe",
        exe,
        "--prefix",
        prefix.to_str().unwrap(),
    ]);
    let shared = confirmed_uninstall("yes\n", true);
    assert!(!shared.status.success());
    assert!(String::from_utf8_lossy(&shared.stderr).contains("shared"));
    sandbox.ok(&["game", "remove", "shared"]);
    fs::write(sandbox.path("uninstall-mode"), "success").unwrap();
    let removed = confirmed_uninstall("yes\n", true);
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(!entry.exists() && !config.exists() && !prefix.exists());
}

#[test]
fn game_help_and_completions_expose_the_new_namespace() {
    let sandbox = Sandbox::new();
    let help = sandbox.ok(&["game", "--help"]);
    let help = String::from_utf8(help.stdout).unwrap();
    let completion = sandbox.ok(&["completions", "zsh"]);
    let completion = String::from_utf8(completion.stdout).unwrap();
    for command in ["install", "uninstall", "create-shortcut", "remove-shortcut"] {
        assert!(help.contains(command));
        assert!(completion.contains(command));
    }
    assert!(!sandbox.drabi(&["desktop", "--help"]).status.success());
}

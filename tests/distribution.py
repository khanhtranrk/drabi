"""Offline smoke tests for the distribution scripts, including terminal confirmation."""

import errno
import hashlib
import os
import pathlib
import pty
import select
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
ASSET = "drabi-x86_64-unknown-linux-gnu"
LATEST = "https://github.com/khanhtranrk/drabi/releases/latest/download/"


def executable(path, text):
    path.write_text(text)
    path.chmod(0o755)


def run(script, env, *args):
    return subprocess.run(
        ["sh", str(ROOT / script), *args],
        env=env,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=15,
    )


def confirm_purge(env, answer):
    pid, terminal = pty.fork()
    if pid == 0:
        command = 'cat "$1" | sh -s -- --purge'
        os.execvpe("sh", ["sh", "-c", command, "sh", str(ROOT / "uninstall.sh")], env)

    details = b""
    answered = False
    deadline = time.monotonic() + 15
    try:
        while True:
            wait = deadline - time.monotonic()
            if wait <= 0 or not select.select([terminal], [], [], wait)[0]:
                raise TimeoutError(details.decode(errors="replace"))
            try:
                chunk = os.read(terminal, 4096)
            except OSError as error:
                if error.errno == errno.EIO:
                    break
                raise
            if not chunk:
                break
            details += chunk
            if not answered and b"Type yes to continue:" in details:
                os.write(terminal, answer.encode() + b"\n")
                answered = True
    except BaseException:
        os.kill(pid, 9)
        raise
    finally:
        os.close(terminal)
        _, status = os.waitpid(pid, 0)
    assert answered, details
    return os.waitstatus_to_exitcode(status)


def test_distribution(release_binary=None):
    with tempfile.TemporaryDirectory(prefix="drabi-distribution-") as directory:
        base = pathlib.Path(directory)
        home, tools, release = (base / name for name in ("home", "tools", "release"))
        for path in (home, tools, release):
            path.mkdir()
        env = os.environ | {
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(base / "xdg-config"),
            "XDG_DATA_HOME": str(base / "xdg-data"),
            "PATH": f"{tools}:/usr/bin:/bin",
            "FIXTURE_RELEASE": str(release),
            "FIXTURE_LATEST": LATEST,
        }
        executable(
            tools / "uname",
            '#!/bin/sh\ncase "$1" in -s) echo Linux;; -m) echo "${FIXTURE_ARCH:-x86_64}";; esac\n',
        )
        executable(
            tools / "gh",
            """#!/bin/sh
case "$1:$2" in
    auth:status) test "${FIXTURE_AUTH:-0}" = 1 ;;
    release:download)
        shift 2
        while [ "$#" -gt 0 ]; do
            case "$1" in
                --dir) directory=$2 ;;
                --repo) [ "$2" = khanhtranrk/drabi ] || exit 2 ;;
                --pattern) [ "$2" = SHA256SUMS ] || asset=$2 ;;
                *) exit 2 ;;
            esac
            shift 2
        done
        cp "$FIXTURE_RELEASE/SHA256SUMS" "$FIXTURE_RELEASE/$asset" "$directory/"
        ;;
    *) exit 2 ;;
esac
""",
        )
        executable(
            tools / "curl",
            """#!/bin/sh
for arg do
    if [ "${previous:-}" = -o ]; then output=$arg; fi
    previous=$arg
done
case "$previous" in
    "$FIXTURE_LATEST"S*|"$FIXTURE_LATEST"drabi-*)
        cp "$FIXTURE_RELEASE/${previous##*/}" "$output"
        ;;
    *) exit 2 ;;
esac
""",
        )
        binary = home / ".local/bin/drabi"

        def prepare(version, asset=ASSET):
            payload = f"#!/bin/sh\nprintf '%s\\n' '{version}'\n"
            (release / asset).write_text(payload)
            digest = hashlib.sha256(payload.encode()).hexdigest()
            (release / "SHA256SUMS").write_text(f"{digest}  {asset}\n")

        def installed(version):
            output = subprocess.check_output([str(binary)], text=True)
            assert output.strip() == version

        prepare("first")
        result = run("install.sh", env)
        assert result.returncode == 0, result.stderr
        installed("first")
        original_inode = binary.stat().st_ino

        prepare("second")
        result = run("install.sh", env)
        assert result.returncode == 0, result.stderr
        installed("second")
        assert binary.stat().st_ino != original_inode

        (release / "SHA256SUMS").write_text(f"{'0' * 64}  {ASSET}\n")
        result = run("install.sh", env)
        assert result.returncode != 0 and "SHA-256 mismatch" in result.stderr
        installed("second")

        (release / ASSET).unlink()
        result = run("install.sh", env)
        assert result.returncode != 0 and "Could not download" in result.stderr
        installed("second")
        assert not list(binary.parent.glob(".drabi-install.*"))

        arm_asset = "drabi-aarch64-unknown-linux-gnu"
        prepare("arm fixture", arm_asset)
        result = run("install.sh", env | {"FIXTURE_ARCH": "aarch64", "FIXTURE_AUTH": "1"})
        assert result.returncode == 0, result.stderr
        installed("arm fixture")

        config = base / "xdg-config/drabi"
        config.mkdir(parents=True)
        (config / "config.toml").touch()
        data = base / "xdg-data/drabi"
        app_data = data / "umu"
        app_data.mkdir(parents=True)
        (app_data / "last-check").touch()
        protected = data / "icons/user-prefix/drive_c"
        protected.mkdir(parents=True)
        wine = home / ".wine"
        wine.mkdir()
        (wine / "system.reg").touch()
        (data / "logs").symlink_to(wine)
        managed = data / "runners/proton/GE-Proton99-1"
        template = managed / "files/share/default_pfx"
        template.mkdir(parents=True)
        (template / "system.reg").touch()
        (managed / ".drabi-managed").touch()
        external = data / "runners/proton/Manual"
        external.mkdir()

        result = run("uninstall.sh", env)
        assert result.returncode == 0 and not binary.exists(), result.stderr
        assert config.exists() and app_data.exists()

        result = run("install.sh", env | {"FIXTURE_ARCH": "aarch64", "FIXTURE_AUTH": "1"})
        assert result.returncode == 0, result.stderr
        assert run("uninstall.sh", env, "--purge").returncode != 0
        assert binary.exists()
        assert confirm_purge(env, "no") != 0
        assert binary.exists() and config.exists() and app_data.exists()

        assert confirm_purge(env, "yes") == 0
        assert not binary.exists() and not config.exists() and not app_data.exists()
        assert not managed.exists()
        assert protected.exists() and external.exists()
        assert (wine / "system.reg").exists() and (data / "logs").is_symlink()
        if release_binary:
            asset = f"drabi-{os.uname().machine}-unknown-linux-gnu"
            payload = release_binary.read_bytes()
            (release / asset).write_bytes(payload)
            digest = hashlib.sha256(payload).hexdigest()
            (release / "SHA256SUMS").write_text(f"{digest}  {asset}\n")
            result = run("install.sh", env | {"FIXTURE_ARCH": os.uname().machine})
            assert result.returncode == 0, result.stderr
            output = subprocess.check_output([str(binary), "--version"], text=True)
            assert output.startswith("drabi "), output
    print("Distribution smoke tests passed")


if __name__ == "__main__":
    test_distribution(pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None)

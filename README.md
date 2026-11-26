# drabi

drabi is a Linux command-line launcher for Windows games and installers. It uses Proton and a user-local UMU Launcher, saves one TOML file per game, and can add games to your application menu.

## Install on Linux

Release binaries support x86-64 and AArch64 (ARM64) on Linux with GNU libc 2.35 or newer. Ubuntu 22.04 and compatible newer distributions can use these binaries. The scripts use standard Linux command-line tools and install into `~/.local/bin` as your user.

### Installation and updates

For a public repository, install or update to the latest stable release with the same command:

```sh
curl -fsSL https://raw.githubusercontent.com/khanhtranrk/drabi/main/install.sh | sh
```

Drabi's GitHub repository is currently private. Sign in with `gh auth login` using an account with access, then fetch the script through the authenticated API:

```sh
gh api -H 'Accept: application/vnd.github.raw+json' \
  'repos/khanhtranrk/drabi/contents/install.sh?ref=main' | sh
```

The installer uses authenticated GitHub CLI downloads when signed in. It verifies SHA-256 before atomically replacing any installed binary. Run the same installer again to update **Drabi itself**. The `drabi update` subcommand updates UMU Launcher.

### Uninstall

For a public repository, remove the executable with:

```sh
curl -fsSL https://raw.githubusercontent.com/khanhtranrk/drabi/main/uninstall.sh | sh
```

To remove Drabi's generated configuration and data as well, run the following and type `yes` at the terminal prompt:

```sh
curl -fsSL https://raw.githubusercontent.com/khanhtranrk/drabi/main/uninstall.sh | sh -s -- --purge
```

For the private repository, use the authenticated API. Omit `-s -- --purge` to remove only the executable.

```sh
gh api -H 'Accept: application/vnd.github.raw+json' \
  'repos/khanhtranrk/drabi/contents/uninstall.sh?ref=main' | sh -s -- --purge
```

Close Drabi-managed games and tools before purging. Purge removes known Drabi-owned files under the XDG configuration and data directories. It preserves external runners and Wine prefixes. Purge skips symlinked data roots and leaves symlink targets outside the Drabi directories intact. Remove any application-menu shortcuts with `drabi game remove-shortcut <id>` before uninstalling if you want to remove those too.

### Download a binary manually

Open [GitHub Releases](https://github.com/khanhtranrk/drabi/releases), sign in if required, and download `SHA256SUMS` and the binary for your architecture to the same directory.

| Linux architecture | Release asset |
| --- | --- |
| x86-64 | `drabi-x86_64-unknown-linux-gnu` |
| AArch64 | `drabi-aarch64-unknown-linux-gnu` |

From the download directory, verify the binary before installation. This example uses x86-64.

```sh
asset=drabi-x86_64-unknown-linux-gnu
awk -v asset="$asset" '$2 == asset' SHA256SUMS | sha256sum --check &&
  mkdir -p "$HOME/.local/bin" &&
  install -m 0755 "$asset" "$HOME/.local/bin/drabi"
```

## Quick start

With `drabi` installed and on your PATH, pick a runner and create a prefix. Proton downloads use curl. UMU needs a compatible Python installation and a host that can run the Steam Linux Runtime.

```sh
drabi proton search
drabi proton install latest
drabi proton list

drabi prefix create "$HOME/Games/demo-prefix"
drabi run "$HOME/Downloads/Game Setup.exe" --prefix "$HOME/Games/demo-prefix"

drabi game add --id demo --name "Demo Game" \
  --exe "$HOME/Games/demo-prefix/drive_c/Games/Demo/Game.exe" \
  --prefix "$HOME/Games/demo-prefix"

drabi run demo --dry-run
drabi run demo
drabi game create-shortcut demo
```

Use the actual location of the installer and the game executable. drabi selects the first available runner unless you select one for the launch or [set a default](examples/config.toml). If you already play the game through another launcher, follow [Reuse an existing installation](docs/user-guide.md#reuse-an-existing-installation).

The first prefix creation or launch can take several minutes while UMU provisions the Steam Linux Runtime.

## Commands

| Command | What it does |
| --- | --- |
| `drabi game add` | Prompt for an ID, name and executable. Pass `--id`, `--name` and `--exe` for scripting. |
| `drabi game install <installer.exe>` | Run a Windows installer and register the detected application. |
| `drabi game uninstall <id>` | Confirm and run the registry uninstaller. Add `--purge` for a game-owned isolated prefix. |
| `drabi game list` | List saved games. |
| `drabi game show <id>`, `game edit <id>`, `game remove <id>` | View, edit or remove a game registration. The editor comes from `VISUAL`, `EDITOR` or `vi`. |
| `drabi game playtime <id>` | Show locally recorded seconds for a game with tracking enabled. |
| `drabi run <game-id-or-exe>` | Start a game or executable and return the UMU exit status. |
| `drabi proton list`, `proton scan` | List local runners. `list` includes external registrations. |
| `drabi proton list --available` | List installable versions from GE, DW, UMU and Steam. Filter with `--source` and one of `ge`, `dw`, `umu` or `valve`. |
| `drabi proton search [query]` | Search one family with `--source`. Defaults to GE. |
| `drabi proton install <version-or-latest>` | Install an exact release or a family alias such as `dw`. Valve builds install through Steam. |
| `drabi proton update <version-or-family>` | Install the newest release from GE, DW or UMU alongside existing versions. Steam updates its own builds. |
| `drabi proton remove <version>` | Remove a version installed by drabi. |
| `drabi proton register <name> <path>`, `proton unregister <name>` | Add or remove a reference to a runner installed elsewhere. |
| `drabi prefix list`, `prefix inspect [path]`, `prefix create [path]` | List, inspect or initialize Wine prefixes. |
| `drabi prefix winecfg [path]`, `prefix winetricks <verb>... [--prefix <path>]` | Use Wine configuration or Winetricks through the selected Proton runner. |
| `drabi prefix regedit [path]`, `prefix control [path]` | Open Registry Editor or Control Panel in the selected prefix. |
| `drabi game create-shortcut <id>`, `game remove-shortcut <id>` | Create, refresh or remove an application-menu shortcut. Creation detects embedded icons and supports `--dry-run`. |
| `drabi doctor`, `update`, `version` | Check local setup, check for a new UMU Launcher, or print the drabi version. |

Use `--format json` on list and query commands to consume output from a script. Use `drabi <command> --help` for options, or `drabi completions zsh` to generate shell completions. See the [user guide](docs/user-guide.md) for runner selection, shared prefixes, menu shortcuts, graphics options, lifecycle hooks, playtime and troubleshooting.

## Publish a release

Update the package version in `Cargo.toml` and refresh `Cargo.lock`. Commit that change before tagging it. Use the version you intend to publish:

```sh
git tag -a v0.2.0 -m "Drabi v0.2.0"
git push origin v0.2.0
```

The [release workflow](.github/workflows/release.yml) runs on version tags. Both Ubuntu 22.04 builds must pass formatting, Clippy, Rust tests and distribution smoke tests. The publish job creates checksums and a release with generated notes. It preserves existing releases and assets. A tag such as `v0.2.0-rc.1` creates a prerelease. Ordinary branch pushes do not publish releases.

## Configuration and development

Game files live in `${XDG_CONFIG_HOME:-$HOME/.config}/drabi/games`. See [config.toml](examples/config.toml) for global defaults and [demo.toml](examples/demo.toml) for a game configuration. The file stem must match the saved game ID. UMU and downloaded Proton versions live in `${XDG_DATA_HOME:-$HOME/.local/share}/drabi`.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Use `cargo install --path . --locked` if you want to build and install from source. Run `python3 tests/distribution.py` for the offline installer and uninstaller smoke tests.

See [AGENTS.md](AGENTS.md) for the code map and [project context](docs/context.md) for behavior, verification and open questions.

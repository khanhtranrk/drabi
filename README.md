# drabi

drabi is a Linux command-line launcher for Windows games and installers. It uses Proton and a user-local UMU Launcher, saves one TOML file per game, and can add games to your application menu.

## Quick start

Build with Rust 1.89 or later. The download commands use curl. UMU needs a compatible Python installation and a host that can run the Steam Linux Runtime.

```sh
cargo install --path . --locked

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
drabi desktop create demo
```

Use the actual location of the installer and the game executable. A single available runner is selected automatically. With several runners, use `--runner <version>` or set a default runner. If you already play the game through another launcher, follow [Reuse an existing installation](docs/user-guide.md#reuse-an-existing-installation).

The first prefix creation or launch can take several minutes while UMU provisions the Steam Linux Runtime.

## Commands

| Command | What it does |
| --- | --- |
| `drabi game add` | Prompt for an ID, name and executable. Pass `--id`, `--name` and `--exe` for scripting. |
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
| `drabi prefix list`, `prefix inspect <path>`, `prefix create <path>` | List, inspect or initialize Wine prefixes. |
| `drabi prefix winecfg <path>`, `prefix winetricks <path> <verb>` | Use Wine configuration or Winetricks through the selected Proton runner. |
| `drabi desktop create <id>`, `desktop remove <id>` | Create, refresh or remove an application-menu shortcut. |
| `drabi doctor`, `update`, `version` | Check local setup, check for a new UMU Launcher, or print the drabi version. |

Use `drabi <command> --help` for options. See the [user guide](docs/user-guide.md) for runner selection, shared prefixes, menu shortcuts, graphics options, playtime and troubleshooting.

## Configuration and development

Game files live in `${XDG_CONFIG_HOME:-$HOME/.config}/drabi/games`. See [demo.toml](examples/demo.toml) for a sample. The file stem must match the saved game ID. UMU and downloaded Proton versions live in `${XDG_DATA_HOME:-$HOME/.local/share}/drabi`.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

See [AGENTS.md](AGENTS.md) for the code map and [project context](docs/context.md) for behavior, verification and open questions.

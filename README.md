# drabi

drabi is a Linux CLI that launches Windows games and installers through Proton and a user-local installation of UMU Launcher. Each saved game has its own TOML file. Commands and options use clap, with plain-text output.

## Quick start

Build with Rust 1.89 or later. The first real launch or explicit update needs network access, curl, and a host that can run the official UMU zipapp and Steam Linux Runtime. Start with an unpacked UMU-compatible Proton runner that contains an executable `proton` script.

```sh
cargo install --path . --locked

drabi proton register ge "$HOME/Tools/GE-Proton"
drabi proton list
drabi prefix create "$HOME/Games/demo-prefix" --runner ge

drabi run "$HOME/Downloads/Game Setup.exe" --runner ge --prefix "$HOME/Games/demo-prefix"
drabi game add --id demo --name "Demo Game" \
  --exe "$HOME/Games/Demo Game/Game.exe" --runner ge --prefix "$HOME/Games/demo-prefix"
drabi run demo --dry-run
drabi run demo
```

Set the executable path to its actual location after installation. Installers can put it under the prefix's `drive_c` directory. For the default prefix, use `drabi prefix create "$HOME/.wine"` if that directory is absent or empty. An existing initialized prefix can be inspected first. Proton can update files within a prefix when it runs.

## Commands

| Command | Behavior |
| --- | --- |
| `drabi run <target>` | Run a saved game ID or an arbitrary `.exe` and return the UMU exit status. Use `./setup.exe` to identify a local file explicitly. |
| `drabi game add` | Prompt for ID, name and executable in a terminal. Supply `--id`, `--name` and `--exe` for scripting. Optional flags set the runner, prefix and working directory. |
| `drabi game list/show/edit/remove` | Manage registrations. Pass an ID to `show`, `edit` or `remove`. The editor is selected using `VISUAL`, then `EDITOR`, then `vi`. Removal only deletes the registration file. |
| `drabi proton scan/list` | Scan the local Proton directory. `list` also includes external registrations. |
| `drabi proton register <name> <path>` | Register a runner directory outside the Proton directory. |
| `drabi proton unregister <name>` | Remove the external entry from configuration while preserving its files. |
| `drabi prefix inspect/create <path>` | Check initialization or explicitly create a prefix. Creation accepts `--runner`. |
| `drabi doctor` | Report locally detectable problems with UMU, runner paths and saved games. |
| `drabi update` | Install or check for a new official UMU Launcher release immediately. |
| `drabi version` | Print the CLI version. |

Use `drabi run --help` for flags. CLI flags override saved values. Repeat `--arg` to append Windows arguments and use `--arg=-flag` for arguments that start with a dash. Repeat `--env KEY=VALUE` to override environment entries. Dry runs display launch details and environment key names and perform no downloads or writes.

## Configuration

The config directory is `${XDG_CONFIG_HOME:-$HOME/.config}/drabi`. Game registrations live in `games/<id>.toml`. Set `default_runner = "ge"` in `config.toml` to choose a global runner. With exactly one available runner, drabi selects it automatically.

Copy the [sample game configuration](examples/demo.toml) to `games/demo.toml` in that directory. Adjust the executable, working directory, runner and prefix for your installation. The filename stem must match `[game].id`.

The working directory defaults to the executable's parent and can be set with `working_dir` under `[game]`. Use absolute paths or paths starting with `~/` in game TOML. The optional `store` field under `[umu]` is passed to UMU.

The first prefix creation or launch can take several minutes while UMU provisions the Steam Linux Runtime.

UMU is installed under `${XDG_DATA_HOME:-$HOME/.local/share}/drabi/umu`. drabi checks for an update at most once per day during normal execution when an installation exists. A failed automatic update leaves the active version usable. Runner discovery reads immediate subdirectories of the drabi data directory's `runners/proton`.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

See [AGENTS.md](AGENTS.md) for the code map and [project context](docs/context.md) for current scope and remaining work.

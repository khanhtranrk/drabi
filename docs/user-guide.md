# Using drabi

## Reuse an existing installation

Find the executable, prefix and runner in your existing launcher's settings. A Proton runner folder should contain an executable file named `proton`. Register the folder under a short name, then register the game with the same prefix and runner.

```sh
drabi proton register dw "$HOME/.local/share/Steam/compatibilitytools.d/DW-Proton Latest"
drabi prefix inspect "$HOME/.wine"
drabi game add --id my-game --name "My Game" \
  --exe "$HOME/Games/My Game/Game.exe" --prefix "$HOME/.wine" --runner dw
drabi run my-game --dry-run
```

You can pass the full runner path instead of registering an alias. drabi stores a game registration separately and uses the original game files. Two games can share a Wine prefix, as a launcher and a game often do. Proton and Winetricks can change files in a shared prefix, so keep track of which runner the other launcher uses.

## Pick and update Proton

```sh
drabi proton search --limit 5
drabi proton install latest
drabi proton list
drabi proton update GE-Proton
```

Use `drabi proton list --available` for installable versions grouped by source. Add `--source dw`, `--source umu`, `--source ge` or `--source valve` to show one family. The default limit is five versions per family. Install a listed version by its exact name, or install the newest stable release with `drabi proton install dw`, `drabi proton install umu` or `drabi proton install ge`.

GE-Proton, DW-Proton and UMU-Proton archives are verified and installed by drabi. Valve Proton and Proton Experimental are distributed by Steam. For example, `drabi proton install proton-experimental` opens an installation request in Steam. After confirming, run `drabi proton list` to see builds installed in the default Steam library and choose `--runner proton-experimental`. For a custom Steam library, use the Proton directory as the runner path or register it with `drabi proton register`. Steam handles updates and removal for its builds.

Search lists matching stable builds from the latest 100 upstream releases. Copy the version from search to install a particular build. Downloaded versions are kept in separate directories and existing game settings keep their version. After updating, use `drabi game edit <id>` to try the new version for one game. Use `drabi proton remove <version>` to remove an unused version after its games exit. drabi blocks removal during launch sessions and prefix commands it starts. Versions explicitly selected by a saved game, the global default or an external registration are protected from removal. `drabi proton unregister <name>` only removes an external reference.

Set defaults in `${XDG_CONFIG_HOME:-$HOME/.config}/drabi/config.toml`. See [the sample config](../examples/config.toml). For both prefix and runner, a CLI flag takes precedence over the saved game setting, then the global default applies. drabi uses `~/.wine` when no prefix is specified or configured.

Without a runner selection, drabi picks the first available runner in this order:

1. Registered runners, sorted by alias.
2. Runners in the Drabi data directory, sorted by directory name.
3. Detected Valve tools in Steam catalog order.

Set `default_runner` or save a runner for the game to keep using the same runner as your installed versions change.

## Set up a new game

1. Create an empty or new prefix path with `drabi prefix create "$HOME/Games/my-prefix" --runner <name>`.
2. Start the installer with `drabi run ./setup.exe --prefix "$HOME/Games/my-prefix" --runner <name>`.
3. Locate the final game executable and pass it to `drabi game add --exe`. You can run `drabi game add` from a terminal to enter the required fields interactively. Set a custom prefix or runner with flags or edit the game file afterward.
4. Run `drabi run <id> --dry-run` to check paths and arguments, then run `drabi run <id>` to start the game.

Use `drabi run <id> --dry-run --show-env` to view the assembled command as an argument list, the working directory and environment values set by drabi. The preview leaves the inherited shell environment undisclosed. Leave out `--show-env` when sharing a preview that could contain private environment values.

`drabi run` expects the selected prefix to be initialized. `prefix list` combines the default, paths used by saved games and prefixes previously created by drabi. Choose a runner that supports Winetricks before running a verb. A Winetricks verb can make lasting changes to the prefix.

These commands use your default prefix and runner when omitted. `inspect` only checks the prefix and needs no runner.

```sh
drabi prefix inspect
drabi prefix create
drabi prefix winecfg
drabi prefix regedit
drabi prefix control
drabi prefix winetricks list-installed
```

Run `prefix create` when the selected prefix is new. To use another prefix, pass its path after the subcommand. For Winetricks use `--prefix <path>`, for example `drabi prefix winetricks list-installed --prefix "$HOME/Games/my-prefix"`. A path with a slash or an existing directory followed by verbs is also accepted in the old positional syntax. Use `--runner <name-or-path>` to override the runner. drabi runs multiple Winetricks verbs separately, reports each failure and returns the first nonzero status.

For Steam-style compatdata with a real `pfx/` directory, pass the parent compatdata path. A normal prefix and its `pfx -> .` link are supported. If `pfx` points to another directory, drabi stops because UMU would replace the link. Inspect the destination and select the intended prefix before retrying. If initialization fails and leaves files behind, inspect the partial directory or create a prefix at a new empty path.

## Add a game to your application menu

Install the CLI into its final location, then run:

```sh
drabi desktop create my-game --name "My Game" --icon "$HOME/Pictures/my-game.png"
```

drabi uses `--icon` first, then `[desktop].icon`, then the first embedded icon from the game's Windows executable. Extracted icons are saved as PNG under `${XDG_DATA_HOME:-$HOME/.local/share}/drabi/icons`. If extraction fails or the executable has no icon, the menu uses its default icon.

Use `drabi desktop create my-game --dry-run` to preview the icon source, output path and desktop entry without writing files.

The entry opens the saved game by ID and is stored in `${XDG_DATA_HOME:-$HOME/.local/share}/applications`. Run `desktop create` again to refresh the entry, or `desktop remove my-game` to delete it. Removing the game registration prints a reminder if its shortcut still exists. If the binary moves, regenerate the entry from the new location. Settings in `[desktop]` are used by future generations, for example:

```toml
[desktop]
name = "My Game"
icon = "~/Pictures/my-game.png"
```

## Read output from scripts

Lists display aligned tables by default. Use `--format json` when another program needs to read the result:

```sh
drabi game list --format json
drabi game show my-game --format json
drabi proton list --available --format json
drabi prefix inspect --format json
```

The option also works with `game playtime`, `proton list`, `proton scan`, `proton search` and `prefix list`. List commands return JSON arrays, including an empty array for no matches. Single-item commands return objects. Diagnostics go to standard error. Check the exit status before using output. A catalog request can return available sources with warnings if another source is unreachable. Plain `game show` still prints the TOML file.

## Tune one game

Try GameMode or Gamescope for a single run with the matching flag. Replace `my-game` with your saved game ID.

```sh
drabi run my-game --gamemode
drabi run my-game --gamescope
drabi run my-game --gamemode --gamescope --dry-run --show-env
```

In the preview, read `Command argv` from left to right. With both tools enabled, the command starts with `gamemoderun`, followed by `gamescope` and its options, then `umu-run` and the game executable. GameMode appears as the `gamemoderun` wrapper in the command.

To save settings for this game, run `drabi game edit my-game` and add or update these tables.

```toml
[performance]
gamemode = true
mangohud = true

[gamescope]
enabled = true
fullscreen = true
width = 1920
height = 1080
render_width = 1280
render_height = 720
refresh_rate = 60
arguments = []
```

In this example, Gamescope presents a 1920 × 1080 output with a 1280 × 720 game resolution and a 60 Hz refresh rate. To use Gamescope with defaults, set only `enabled = true` under `[gamescope]`.

Use `--mangohud` to enable MangoHud for one run. Use the matching `--no-gamescope`, `--no-gamemode` or `--no-mangohud` flag to disable a saved setting for one run. GameMode needs `gamemoderun` on your PATH. MangoHud with Gamescope uses `mangoapp`.

If an enabled tool is missing, both a normal launch and a dry run stop before opening the game. For example, enabling GameMode without `gamemoderun` reports `gamemoderun is enabled but missing from PATH`. Run `drabi doctor` to check which tools are available.

Saved executable arguments go in `[game].arguments`. Use `--arg=-flag` for an argument starting with a dash. Put custom environment variables in `[environment]` or pass `--env KEY=VALUE` on the command line.

## Enable playtime or detach a launch

Playtime is off by default. To track a saved game, add:

```toml
[lifecycle]
track_playtime = true
```

View the total with `drabi game playtime <id>`. The total measures wall-clock time spent waiting for the launch process and is stored locally. A managed game can also specify a trusted local command and arguments with `post_exit = ["/path/to/script", "argument"]` under `[lifecycle]`. The hook runs in the game's working directory and receives the exit code in `DRABI_EXIT_CODE`.

Use `drabi run <id> --detached` or save `detached = true` under `[lifecycle]` to return after starting the child in a new session. The terminal prints the log path. Playtime and post-exit commands require managed mode. Use `--managed` to override a saved detached setting.

## Troubleshoot

| Symptom | What to check |
| --- | --- |
| Missing prefix | Confirm the path with `prefix inspect`. Create it explicitly if it is new. |
| Runner is missing or the wrong one was chosen | Try `proton list` and pin a version with `--runner` or edit the game configuration. |
| First launch takes a while | UMU can need to download and unpack the Steam Linux Runtime. Run from a terminal to view progress. |
| Game works in another launcher | Compare the executable, prefix, runner version, working directory, environment and arguments with `run <id> --dry-run`. |
| Shortcut opens nothing | Try the same game ID in a terminal and consult `drabi doctor`. Regenerate the shortcut if drabi moved. |
| An optional tool is enabled but missing | Install it via your distribution or launch with the matching `--no-...` flag. |

For supported games, drabi passes through Proton's EAC and BattlEye runtime variables and discovers standard Steam installations of those runtimes when present. Set `PROTON_EAC_RUNTIME` or `PROTON_BATTLEYE_RUNTIME` in the game environment for a custom location. Compatibility depends on the game developer and the selected Proton build.

# Working on drabi

Read [README.md](README.md) and the [user guide](docs/user-guide.md) for CLI behavior and [docs/context.md](docs/context.md) for requirements, verification and remaining work. See the [runtime decision](docs/decisions/0001-delegate-runtime-to-umu.md) before changing launch or update behavior.

## Code map

| File | Responsibility |
| --- | --- |
| `src/main.rs` | clap commands, game registration and editing, diagnostics |
| `src/config.rs` | XDG paths, TOML models and atomic writes |
| `src/installation.rs` and `src/installation/discovery.rs` | Installer and uninstaller flow, Wine registry, shortcut and executable discovery |
| `src/desktop.rs` | XDG application-menu shortcut formatting and storage |
| `src/prefixes.rs` | Known prefix paths and Wine utility invocation |
| `src/releases.rs` | GE-Proton release search and verified managed installs |
| `src/performance.rs` | Tool detection and Linux launch wrappers |
| `src/lifecycle.rs` | Detached sessions, opt-in playtime and post-exit hooks |
| `src/runner.rs` | Proton discovery, validation and external registration |
| `src/runtime.rs` | Official UMU download, validation and activation |
| `src/launch.rs` | Launch precedence, argv and environment, prefix initialization |
| `tests/cli.rs` | Isolated CLI regression tests with a local runtime fixture |

## Invariants

- Keep one TOML file per game. Validate IDs before deriving filenames.
- Keep `run --dry-run` read-only and offline.
- Check prefix initialization before launching an executable. Only explicit `prefix create` and `game install` request initialization.
- Preserve game, prefix and external runner files when removing registrations.
- Use direct process arguments and an absolute resolved Proton directory. Keep UMU Launcher user-local.
- Exercise CLI changes through the normal executable with temporary XDG directories. Run the checks listed in the README and update the docs when behavior changes.

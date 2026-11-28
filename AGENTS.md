# Working on drabi

## Mandatory contribution workflow

Follow [CONTRIBUTING.md](CONTRIBUTING.md) before making changes. Reuse or create an issue, work on a dedicated branch, validate changes, and open or update a linked PR targeting the default branch. Never commit or push directly to the default branch, merge a PR, or bypass the workflow. Use English for repository-facing contributions. Report blockers and actual validation results. Leave merging to the maintainer.


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

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- When graphify-out/graph.json exists, run `graphify query "<question>"` before your first grep, search or source-file read for a codebase question, and put this rule in every subagent prompt that explores code. Read raw files only for lines the graph or an earlier read already located. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
- A committed graphify-out/graph.json is rewritten whole on every build, so a large diff to it in a pull request is expected. Do not flag it as churn or suggest skipping the graph rebuild or its hooks.

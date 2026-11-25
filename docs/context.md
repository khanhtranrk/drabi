# drabi project context

## Goal

Provide a lightweight Linux launcher for Windows games and installers. The first milestone is an end-to-end launch through a user-local UMU Launcher, a compatible Proton runner and an explicitly initialized prefix.

## MVP acceptance flow

1. Register an unpacked Proton distribution or discover one under the data directory.
2. Run `prefix create` with the selected runner.
3. Launch an installer whose path includes spaces with that runner and prefix.
4. Register the installed executable with the same prefix and launch it by ID.
5. Remove the registration and verify that the executable, prefix and runner remain on disk.

The CLI regression suite substitutes a local executable for UMU and uses temporary XDG directories. The tests cover argv boundaries, environment, child exit status, explicit prefix creation, invalid editor changes, file preservation and offline UMU fallback. The release bootstrap was exercised against the official UMU 1.4.4 zipapp in a temporary XDG data directory. A Linux smoke test used DW-Proton 11.0-17 and Steam Runtime 4 to create and inspect a temporary prefix, run a Windows command executable from a path containing spaces, and run it by registration ID. The ID launch returned the requested Windows exit code 37. Removing the registration preserved the prefix and executable. Use the README flow with a representative Windows game and installer to validate graphics and compatibility.

## Design details

- Saved and explicit runner selectors resolve to absolute Proton directories. A single runner can serve as the implicit default. Multiple runners require a selector or a global default.
- The prefix probe recognizes `drive_c` and `system.reg` at either the supplied path or its `pfx` child. This is a structural check. The selected Proton build determines compatibility with existing Wine data.
- UMU is the managed launcher. UMU itself handles Steam Linux Runtime provisioning. See the [runtime decision](decisions/0001-delegate-runtime-to-umu.md).
- Game IDs and UMU IDs are independent. The CLI uses the former to locate game TOML. UMU receives the latter through `GAMEID`.
- Commands report local errors with actionable paths. `doctor` is read-only. It reports configuration and structural checks.

## Remaining milestones

1. Add the GE-Proton release provider, versioned installs, updates, removal and archive verification. Keep external registration independent of managed installs.
2. Add prefix listing, winecfg and Winetricks. Evaluate prefix concurrency and compatibility against real Proton runners.
3. Add Gamescope, GameMode and MangoHud. Validate enabled tool availability and wrapper order on a desktop session.
4. Add desktop entry creation and removal, and validate executable quoting and field-code escaping.
5. Add detached execution, optional playtime and post-exit commands. Validate UMU and Proton compatibility across representative distros and games. EAC and BattlEye support depends on the game and upstream runtime.

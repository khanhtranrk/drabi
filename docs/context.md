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

## Feature follow-up

The GE-Proton, DW-Proton and UMU-Proton providers install verified releases into independent version directories. A marker identifies runners installed by drabi. Unmarked discovered directories and external registrations are preserved. Available versions come from upstream release metadata and a catalog of supported Steam tools. Valve Proton installation is delegated to Steam. Neither changes the game files.

Application-menu entries launch saved game IDs through the absolute drabi binary path. Desktop entry quoting follows the freedesktop string and Exec escaping rules. Prefix tools reuse the managed UMU and selected Proton environment. Gamescope, GameMode and MangoHud are optional host tools. Playtime defaults to disabled and is supported in managed mode.

## Verification limits

CLI integration tests cover isolated launch and update behavior, menu entry generation, release installation and removal, and opt-in lifecycle settings. The Linux verification also downloaded and verified GE-Proton11-14, ran Winetricks through DW-Proton, and launched a generated desktop entry through GIO using a fixture executable. [Issue #6](https://github.com/khanhtranrk/drabi/issues/6) tracks real graphics and game-specific compatibility checks. EAC and BattlEye rely on Proton, the official runtime and game developer support.


## Audit follow-up

Prefix validation follows the directory Proton will use and rejects a `pfx` symlink that redirects to another directory. Managed runner removal takes an exclusive per-version lease. Launches and prefix tools hold a shared lease, with a waiting drabi process holding it for a detached launch. This coordinates drabi processes using the same data directory.

An automatic UMU update that cannot acquire or open its update lock may fall back to the active version. Explicit updates report the failure. A verified replacement can recover an incomplete version directory. The original contents remain under `umu/recovery-*/incomplete`. Installed UMU versions continue to be retained.

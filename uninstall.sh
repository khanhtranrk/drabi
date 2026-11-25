#!/bin/sh

set -eu

fail() {
    printf 'drabi: %s\n' "$*" >&2
    exit 1
}

# Generated files can coexist with manually placed data in the XDG trees.
remove_owned() {
    candidate=$1
    template=${2:-}
    if [ -L "$candidate" ]; then
        printf 'Preserved symlink %s\n' "$candidate" >&2
        return
    fi
    [ -e "$candidate" ] || return 0
    if [ -d "$candidate" ]; then
        markers=$(find "$candidate" \
            \( -path "$template" -prune \) -o \
            \( -name drive_c -o -name dosdevices -o -name system.reg -o -name user.reg -o -name userdef.reg \) \
            -print -quit) || fail "Could not inspect $candidate."
        if [ -n "$markers" ]; then
            printf 'Preserved possible Wine prefix under %s\n' "$candidate" >&2
            return
        fi
    fi
    rm -rf -- "$candidate" || fail "Could not remove $candidate."
}

purge_config() {
    if [ -L "$1" ]; then
        printf 'Preserved symlink %s\n' "$1" >&2
        return
    fi
    [ -d "$1" ] || return 0
    for name in config.toml games prefixes.toml .config.lock; do
        remove_owned "$1/$name"
    done
    rmdir -- "$1" 2>/dev/null || :
}

purge_data() {
    if [ -L "$1" ]; then
        printf 'Preserved symlink %s\n' "$1" >&2
        return
    fi
    [ -d "$1" ] || return 0
    for name in umu icons logs playtime .prefix-create.lock; do
        remove_owned "$1/$name"
    done

    runners="$1/runners"
    proton="$runners/proton"
    if [ ! -L "$runners" ] && [ ! -L "$proton" ] && [ -d "$proton" ]; then
        for runner in "$proton"/*; do
            if [ ! -d "$runner" ] || [ -L "$runner" ] ||
                [ ! -f "$runner/.drabi-managed" ] || [ -L "$runner/.drabi-managed" ]; then
                continue
            fi
            # Proton's shipped template is part of the runner payload.
            remove_owned "$runner" "$runner/files/share/default_pfx"
        done
        remove_owned "$proton/.lock"
        remove_owned "$proton/.leases"
        rmdir -- "$proton" "$runners" 2>/dev/null || :
    fi
    rmdir -- "$1" 2>/dev/null || :
}

main() {
    case ${HOME:-} in
        /|'') fail 'HOME must point to your home directory.' ;;
        /*) ;;
        *) fail 'HOME must be an absolute path.' ;;
    esac
    case "$#:${1:-}" in
        0:) purge=false ;;
        1:--purge) purge=true ;;
        *) fail 'Usage: sh uninstall.sh [--purge]' ;;
    esac

    if [ "$purge" = true ]; then
        case ${XDG_CONFIG_HOME:-} in
            /*) config="$XDG_CONFIG_HOME/drabi" ;;
            *) config="$HOME/.config/drabi" ;;
        esac
        case ${XDG_DATA_HOME:-} in
            /*) data="$XDG_DATA_HOME/drabi" ;;
            *) data="$HOME/.local/share/drabi" ;;
        esac
        printf 'Remove drabi and its configuration/data under these directories?\n  %s\n  %s\nType yes to continue: ' \
            "$config" "$data" >&2
        if ! IFS= read -r answer </dev/tty || [ "$answer" != yes ]; then
            fail 'Purge cancelled. Use a terminal and type yes to confirm.'
        fi
        purge_config "$config"
        purge_data "$data"
    fi

    rm -f -- "$HOME/.local/bin/drabi" || fail 'Could not remove the installed binary.'
    printf 'Removed drabi from %s\n' "$HOME/.local/bin"
}

main "$@"

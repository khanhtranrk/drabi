#!/bin/sh

set -eu

fail() {
    printf 'drabi: %s\n' "$*" >&2
    exit 1
}

main() {
    [ "$#" -eq 0 ] || fail 'Usage: sh install.sh'
    case ${HOME:-} in
        /|'') fail 'HOME must point to your home directory.' ;;
        /*) ;;
        *) fail 'HOME must be an absolute path.' ;;
    esac
    [ "$(uname -s)" = Linux ] || fail 'This installer supports Linux.'
    case $(uname -m) in
        x86_64|amd64) arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) fail 'Supported architectures are x86_64 and aarch64.' ;;
    esac
    command -v sha256sum >/dev/null 2>&1 || fail 'sha256sum is required.'

    repo=khanhtranrk/drabi
    asset="drabi-$arch-unknown-linux-gnu"
    bin_dir="$HOME/.local/bin"
    mkdir -p "$bin_dir" || fail "Cannot create $bin_dir."
    stage=$(mktemp -d "$bin_dir/.drabi-install.XXXXXXXX") || fail 'Cannot stage the download.'
    trap 'rm -rf -- "$stage"' 0
    trap 'exit 1' HUP INT TERM

    if command -v gh >/dev/null 2>&1 && gh auth status --hostname github.com >/dev/null 2>&1; then
        gh release download --repo "$repo" --pattern "$asset" --pattern SHA256SUMS --dir "$stage" ||
            fail 'Could not download the latest stable release. Check your repository access.'
    else
        command -v curl >/dev/null 2>&1 || fail 'Install curl or authenticate GitHub CLI.'
        release="https://github.com/$repo/releases/latest/download"
        for file in SHA256SUMS "$asset"; do
            curl -q -fsSL --proto '=https' --proto-redir '=https' --connect-timeout 10 \
                --max-time 120 -o "$stage/$file" "$release/$file" ||
                fail 'Could not download the latest stable release. For a private repository, sign in with gh auth login.'
        done
    fi

    expected=$(LC_ALL=C awk -v asset="$asset" '
        (NF == 2 && ($2 == asset || $2 == "*" asset)) {
            if (length($1) != 64 || $1 ~ /[^0-9a-fA-F]/) exit 1
            digest = tolower($1)
            count++
        }
        END { if (count != 1) exit 1; print digest }
    ' "$stage/SHA256SUMS") || fail "Missing, invalid or duplicate SHA-256 for $asset."
    actual=$(sha256sum < "$stage/$asset") || fail "Cannot hash $asset."
    [ "${actual%% *}" = "$expected" ] || fail "SHA-256 mismatch for $asset."

    chmod 0755 "$stage/$asset" || fail 'Cannot make the verified binary executable.'
    mv -fT -- "$stage/$asset" "$bin_dir/drabi" || fail "Cannot install $bin_dir/drabi."
    printf 'Installed drabi at %s\n' "$bin_dir/drabi"
    case ":$PATH:" in
        *":$bin_dir:"*) ;;
        *) printf 'Add %s to PATH, or invoke the binary by its full path.\n' "$bin_dir" ;;
    esac
}

main "$@"

#!/bin/sh
# Install or update outlay, the keyboard-driven xrandr layout editor.
#
#   curl -fsSL https://github.com/Papayah/outlay/releases/latest/download/install.sh | sh
#
# Downloads the static binary for this machine from a GitHub release, checks it against the
# release's SHA256SUMS and installs it to ~/.local/bin, with shell completions. Running it again
# updates an older install. See --help for the options; OUTLAY_RELEASES_URL points it at a mirror
# of the releases page.
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# POSIX sh (dash, busybox ash): no `local`, no arrays. Everything is in functions and `main` runs
# on the last line, so a download cut short runs nothing.

set -eu

say() {
    printf 'outlay-install: %s\n' "$*" >&2
}

die() {
    say "error: $*"
    exit 1
}

usage() {
    cat <<'EOF'
Install or update outlay from its GitHub releases.

  curl -fsSL https://github.com/Papayah/outlay/releases/latest/download/install.sh | sh
  curl -fsSL .../install.sh | sh -s -- [OPTIONS]

Options:
  --version X.Y.Z   install this release instead of the latest (an older one downgrades)
  --to DIR          install into DIR instead of ~/.local/bin (skips shell completions)
  --force           reinstall even when that version is installed already
  --no-completions  do not install bash, zsh and fish completions
  --uninstall       remove outlay and its completions; keeps the config and state
  -h, --help        show this help

Environment:
  OUTLAY_RELEASES_URL  releases base URL (default https://github.com/Papayah/outlay/releases)
EOF
}

parse_args() {
    opt_version=
    opt_to=
    opt_force=0
    opt_completions=1
    opt_uninstall=0
    while [ $# -gt 0 ]; do
        case $1 in
            --version)
                [ $# -ge 2 ] || die "--version needs a value, such as 0.1.0"
                opt_version=$2
                shift
                ;;
            --version=*) opt_version=${1#*=} ;;
            --to)
                [ $# -ge 2 ] || die "--to needs a directory"
                opt_to=$2
                shift
                ;;
            --to=*) opt_to=${1#*=} ;;
            --force) opt_force=1 ;;
            --no-completions) opt_completions=0 ;;
            --uninstall) opt_uninstall=1 ;;
            -h | --help)
                usage
                exit 0
                ;;
            *) die "unknown option: $1 (see --help)" ;;
        esac
        shift
    done

    opt_version=${opt_version#v}
    case $opt_version in
        *[!0-9A-Za-z.+-]*) die "not a version: $opt_version" ;;
    esac
}

# Sets dir (absolute, no trailing slash), default_dir and the XDG base directories.
resolve_dirs() {
    if [ -z "$opt_to" ] && [ -z "${HOME:-}" ]; then
        die "HOME is not set; pass --to DIR"
    fi
    default_dir=${HOME:-}/.local/bin
    data_home=${XDG_DATA_HOME:-${HOME:-}/.local/share}
    config_home=${XDG_CONFIG_HOME:-${HOME:-}/.config}
    state_home=${XDG_STATE_HOME:-${HOME:-}/.local/state}

    case $opt_to in
        '') dir=$default_dir ;;
        /*) dir=$opt_to ;;
        *) dir=$PWD/$opt_to ;;
    esac
    while [ "$dir" != / ] && [ "${dir%/}" != "$dir" ]; do
        dir=${dir%/}
    done
    if [ -d "$dir/outlay" ]; then
        die "$dir/outlay is a directory"
    fi
}

is_default_dir() {
    [ "$dir" = "$default_dir" ]
}

# Sets cpath to where the completion file for shell $1 goes.
completion_path() {
    case $1 in
        bash) cpath=$data_home/bash-completion/completions/outlay ;;
        zsh) cpath=$data_home/zsh/site-functions/_outlay ;;
        fish) cpath=$config_home/fish/completions/outlay.fish ;;
    esac
}

# Prints $1 with a leading $HOME written as the literal text $HOME, for lines to paste into an rc
# file.
rc_path() {
    case $1 in
        "${HOME:-/nonexistent}"/*) printf '%s' "\$HOME/${1#"$HOME"/}" ;;
        *) printf '%s' "$1" ;;
    esac
}

# Warns when $dir is not on PATH, or when the shell would run another outlay first. Best-effort:
# call it as `check_path || true`.
check_path() {
    case ":${PATH:-}:" in
        *":$dir:"*) ;;
        *)
            say "$dir is not on your PATH; add this line to your shell's rc file:"
            say "  export PATH=\"$(rc_path "$dir"):\$PATH\""
            ;;
    esac
    found=$(command -v outlay 2>/dev/null) || found=
    if [ -n "$found" ] && [ "$found" != "$dir/outlay" ]; then
        say "warning: the command outlay runs $found, not $dir/outlay; check the order of your PATH"
    fi
}

uninstall() {
    if [ -f "$dir/outlay" ] || [ -L "$dir/outlay" ]; then
        rm -f "$dir/outlay"
        say "removed $dir/outlay"
    else
        say "no outlay in $dir (pass --to DIR if you installed it elsewhere)"
    fi
    if is_default_dir; then
        for sh in bash zsh fish; do
            completion_path "$sh"
            if [ -f "$cpath" ]; then
                rm -f "$cpath"
                say "removed $cpath"
            fi
        done
    fi
    for kept in "$config_home/outlay" "$state_home/outlay"; do
        if [ -d "$kept" ]; then
            say "kept $kept; remove it by hand if you want it gone"
        fi
    done
    found=$(command -v outlay 2>/dev/null) || found=
    if [ -n "$found" ]; then
        say "note: another outlay is still on PATH: $found"
    fi
}

# Sets target to the Rust target triple of the release asset for this machine.
detect_target() {
    os=$(uname -s) || os=unknown
    if [ "$os" != Linux ]; then
        die "outlay runs on Linux with X11; this system is $os"
    fi
    arch=$(uname -m) || arch=unknown
    case $arch in
        x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
        aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
        *) die "there is no prebuilt outlay for $arch; build it from source with: cargo install --git https://github.com/Papayah/outlay --locked" ;;
    esac

    missing=
    for tool in curl tar sha256sum mktemp; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            missing="$missing $tool"
        fi
    done
    if [ -n "$missing" ]; then
        die "missing required tools:$missing"
    fi
}

# fetch URL FILE
fetch() {
    curl -fsSL --retry 3 --proto-redir '=https' -o "$2" "$1"
}

# Sets ver, asset and sum from the release's SHA256SUMS.
pick_release() {
    base=${OUTLAY_RELEASES_URL:-https://github.com/Papayah/outlay/releases}
    base=${base%/}
    if [ -n "$opt_version" ]; then
        sums_url=$base/download/v$opt_version/SHA256SUMS
    else
        sums_url=$base/latest/download/SHA256SUMS
    fi
    if ! fetch "$sums_url" "$tmp/SHA256SUMS"; then
        if [ -n "$opt_version" ]; then
            die "could not download $sums_url; is $opt_version a released version?"
        fi
        die "could not download $sums_url"
    fi

    asset=
    sum=
    while read -r line_sum line_name || [ -n "$line_sum" ]; do
        line_name=${line_name#\*}
        case $line_name in
            outlay-*-"$target".tar.gz)
                asset=$line_name
                sum=$line_sum
                break
                ;;
        esac
    done <"$tmp/SHA256SUMS"
    if [ -z "$asset" ]; then
        die "this release has no build for $target; build it from source with: cargo install --git https://github.com/Papayah/outlay --locked"
    fi
    case $sum in
        *[!0-9a-f]*) die "SHA256SUMS has a malformed line for $asset" ;;
    esac
    if [ ${#sum} -ne 64 ]; then
        die "SHA256SUMS has a malformed line for $asset"
    fi

    ver=${asset#outlay-}
    ver=${ver%-"$target".tar.gz}
    if [ -n "$opt_version" ] && [ "$ver" != "$opt_version" ]; then
        die "the v$opt_version release holds $asset, not version $opt_version"
    fi
}

# Sets old to the installed version: empty when nothing is installed, "?" when it cannot be read.
installed_version() {
    old=
    if [ -f "$dir/outlay" ]; then
        old=$("$dir/outlay" --version 2>/dev/null) || old=
        case $old in
            "outlay "*) old=${old#outlay } ;;
            *) old="?" ;;
        esac
    fi
}

# Downloads, verifies and installs $asset; on any failure the old binary stays as it was.
install_binary() {
    url=$base/download/v$ver/$asset
    fetch "$url" "$tmp/$asset" || die "could not download $url"
    printf '%s  %s\n' "$sum" "$asset" >"$tmp/sum"
    if ! (cd "$tmp" && sha256sum -c sum >/dev/null 2>&1); then
        die "checksum mismatch for $asset: the download is corrupt; nothing was installed"
    fi
    tar -xzf "$tmp/$asset" -C "$tmp" || die "could not unpack $asset"
    src=$tmp/outlay-$ver-$target/outlay
    [ -f "$src" ] || die "$asset holds no outlay-$ver-$target/outlay"

    mkdir -p "$dir" || die "could not create $dir"
    # Staged next to its final name: the version check runs where the binary will live (not in a
    # possibly noexec /tmp), and the mv below is an atomic rename.
    staged=$dir/.outlay.new.$$
    cp "$src" "$staged" || die "could not write to $dir"
    chmod 755 "$staged"
    got=$("$staged" --version 2>/dev/null) || got=
    if [ "$got" != "outlay $ver" ]; then
        die "the new binary does not run here (it printed '${got:-nothing}'); is $dir on a noexec file system?"
    fi
    mv -f "$staged" "$dir/outlay" || die "could not replace $dir/outlay"
    staged=
}

# Generates the completions with the new binary, for the shells this system has.
install_completions() {
    if [ "$opt_completions" = 0 ]; then
        return 0
    fi
    if ! is_default_dir; then
        say "skipped shell completions for --to; generate them with: outlay completions bash|zsh|fish"
        return 0
    fi
    for sh in bash zsh fish; do
        if ! command -v "$sh" >/dev/null 2>&1; then
            continue
        fi
        completion_path "$sh"
        first=1
        if [ -f "$cpath" ]; then
            first=0
        fi
        if "$dir/outlay" completions "$sh" >"$tmp/completion" 2>/dev/null &&
            mkdir -p "${cpath%/*}" && cp "$tmp/completion" "$cpath"; then
            say "$sh completions: $cpath"
        else
            say "warning: could not write the $sh completions to $cpath"
            continue
        fi
        if [ "$sh" = zsh ] && [ "$first" = 1 ]; then
            say "zsh: if completion does not work, add this line to ~/.zshrc before compinit:"
            say "  fpath=(\"$(rc_path "${cpath%/*}")\" \$fpath)"
        fi
    done
}

cleanup() {
    if [ -n "$staged" ]; then
        rm -f "$staged"
    fi
    if [ -n "$tmp" ]; then
        rm -rf "$tmp"
    fi
}

main() {
    parse_args "$@"
    resolve_dirs

    if [ "$opt_uninstall" = 1 ]; then
        uninstall
        return 0
    fi

    if [ -z "$opt_to" ] && [ "$(id -u)" = 0 ]; then
        say "running as root: this installs for root only; pass --to /usr/local/bin to install for everyone"
    fi
    detect_target

    tmp=
    staged=
    trap cleanup EXIT
    trap 'exit 129' HUP
    trap 'exit 130' INT
    trap 'exit 143' TERM
    tmp=$(mktemp -d) || die "could not create a temporary directory"

    pick_release
    installed_version
    if [ "$old" = "$ver" ] && [ "$opt_force" = 0 ]; then
        say "outlay $ver is up to date ($dir/outlay)"
        check_path || true
        return 0
    fi

    install_binary
    case $old in
        '') say "Installed outlay $ver to $dir/outlay" ;;
        "$ver") say "Reinstalled outlay $ver ($dir/outlay)" ;;
        '?') say "Installed outlay $ver to $dir/outlay, replacing a binary that did not report its version" ;;
        *) say "Updated outlay $old -> $ver ($dir/outlay)" ;;
    esac
    install_completions
    check_path || true
    if ! command -v xrandr >/dev/null 2>&1; then
        say "outlay needs the xrandr program: xorg-xrandr on Arch, x11-xserver-utils on Debian and Ubuntu, xrandr elsewhere" || true
    fi
}

main "$@"

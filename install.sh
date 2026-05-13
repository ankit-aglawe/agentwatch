#!/usr/bin/env sh
# agentwatch installer — Linux · macOS · WSL2
#
# Usage:
#   curl -fsSL https://agentwatch.sh | sh
#
# Environment overrides:
#   AGENTWATCH_VERSION       — git tag or "latest" (default: latest)
#   AGENTWATCH_INSTALL_DIR   — destination directory (default: ~/.local/bin)
#   AGENTWATCH_NO_PROMPT     — set to 1 to skip the confirmation prompt
#
# What this does (transparency before piping to shell):
#   1. Detects OS + CPU architecture.
#   2. Resolves the matching prebuilt binary tag from GitHub Releases.
#   3. Downloads tarball + SHA-256 checksum file, verifies the hash.
#   4. Drops the `agentwatch` binary into AGENTWATCH_INSTALL_DIR.
#   5. Prints a one-line "add this to your PATH" if the dir isn't on it.

set -eu

REPO="ankit-aglawe/agentwatch"
VERSION="${AGENTWATCH_VERSION:-latest}"
INSTALL_DIR="${AGENTWATCH_INSTALL_DIR:-${HOME}/.local/bin}"
NO_PROMPT="${AGENTWATCH_NO_PROMPT:-}"

C_CYAN='\033[38;2;148;226;213m'
C_DIM='\033[2m'
C_RESET='\033[0m'

say() { printf "%b%s%b\n" "$C_CYAN" "$1" "$C_RESET"; }
dim() { printf "%b%s%b\n" "$C_DIM"  "$1" "$C_RESET"; }
die() { printf "error: %s\n" "$1" >&2; exit 1; }

detect_target() {
    os_raw=$(uname -s); arch_raw=$(uname -m)
    case "$os_raw" in
        Linux*)  os=unknown-linux-gnu ;;
        Darwin*) os=apple-darwin ;;
        *) die "unsupported OS: $os_raw" ;;
    esac
    case "$arch_raw" in
        x86_64|amd64)   arch=x86_64 ;;
        aarch64|arm64)  arch=aarch64 ;;
        *) die "unsupported architecture: $arch_raw" ;;
    esac
    printf "%s-%s" "$arch" "$os"
}

confirm() {
    [ -n "$NO_PROMPT" ] && return 0
    [ ! -t 0 ] && return 0           # piped / non-interactive: skip prompt
    printf "Press Enter to continue, Ctrl+C to abort: "
    read -r _
}

main() {
    target=$(detect_target)
    say   "agentwatch installer"
    dim   "  version : $VERSION"
    dim   "  target  : $target"
    dim   "  install : $INSTALL_DIR"
    confirm

    mkdir -p "$INSTALL_DIR"

    # TODO: enable once v0.1 is tagged in GitHub Releases.
    # tarball="agentwatch-${VERSION}-${target}.tar.gz"
    # base="https://github.com/${REPO}/releases/download/${VERSION}"
    # curl -fsSLO "${base}/${tarball}"
    # curl -fsSLO "${base}/${tarball}.sha256"
    # sha256sum -c "${tarball}.sha256"
    # tar xzf "$tarball" -C "$INSTALL_DIR" agentwatch
    # rm -f "$tarball" "${tarball}.sha256"

    say "(v0.1 not yet released — track https://github.com/${REPO}/releases)"
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *) dim "note: add $INSTALL_DIR to your PATH" ;;
    esac
}

main "$@"

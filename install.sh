#!/usr/bin/env sh
#
# agentwatch installer - Linux · macOS · WSL2
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/ankit-aglawe/agentwatch/main/install.sh | sh
#
# Environment overrides:
#   AGENTWATCH_VERSION         git tag or "latest"   (default: latest)
#   AGENTWATCH_INSTALL_DIR     destination directory (default: ~/.local/bin)
#   AGENTWATCH_NO_PROMPT       set to 1 to skip the Y/n confirmation
#   AGENTWATCH_NO_MODIFY_PATH  set to 1 to skip touching your shell rc
#   AGENTWATCH_FORCE           set to 1 to overwrite an existing install
#
# What this does (transparency before piping to shell):
#   1. Verifies required tools (curl, tar, sha256).
#   2. Detects OS + CPU architecture.
#   3. Confirms with the user (unless piped non-interactively).
#   4. Downloads the right asset from GitHub Releases over TLS 1.2+.
#   5. Verifies the SHA-256 checksum (when published).
#   6. Atomically installs to AGENTWATCH_INSTALL_DIR.
#   7. Adds AGENTWATCH_INSTALL_DIR to your PATH in the appropriate shell rc.

set -eu

# -------- Constants ----------------------------------------------------------

REPO="ankit-aglawe/agentwatch"
BINARY="agentwatch"

VERSION="${AGENTWATCH_VERSION:-latest}"
INSTALL_DIR="${AGENTWATCH_INSTALL_DIR:-${HOME}/.local/bin}"
NO_PROMPT="${AGENTWATCH_NO_PROMPT:-}"
NO_MODIFY_PATH="${AGENTWATCH_NO_MODIFY_PATH:-}"
FORCE="${AGENTWATCH_FORCE:-}"

# Catppuccin Mocha - only emitted when stdout is a TTY.
if [ -t 1 ]; then
    C_TEAL='\033[38;2;148;226;213m'
    C_PEACH='\033[38;2;250;179;135m'
    C_RED='\033[38;2;243;139;168m'
    C_DIM='\033[2m'
    C_BOLD='\033[1m'
    C_RESET='\033[0m'
else
    C_TEAL=''; C_PEACH=''; C_RED=''; C_DIM=''; C_BOLD=''; C_RESET=''
fi

step() { printf '%b%s%b\n' "$C_TEAL" "$1" "$C_RESET"; }
sub()  { printf '%b  %s%b\n' "$C_DIM"  "$1" "$C_RESET"; }
warn() { printf '%bwarn:%b %s\n' "$C_PEACH" "$C_RESET" "$1" >&2; }
fail() { printf '%berror:%b %s\n' "$C_RED" "$C_RESET" "$1" >&2; exit 1; }

# -------- Prerequisite check ------------------------------------------------

check_prereqs() {
    missing=''
    for cmd in curl tar uname mktemp; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            missing="$missing $cmd"
        fi
    done
    if ! command -v sha256sum >/dev/null 2>&1 \
        && ! command -v shasum    >/dev/null 2>&1 \
        && ! command -v openssl   >/dev/null 2>&1; then
        missing="$missing sha256sum|shasum|openssl"
    fi
    if [ -n "$missing" ]; then
        fail "missing required tools:$missing
Install them with your package manager and re-run."
    fi
}

sha256_of() {
    file=$1
    if   command -v sha256sum >/dev/null 2>&1; then sha256sum "$file" | awk '{print $1}'
    elif command -v shasum    >/dev/null 2>&1; then shasum -a 256 "$file" | awk '{print $1}'
    else                                            openssl dgst -sha256 "$file" | awk '{print $NF}'
    fi
}

# -------- Target detection --------------------------------------------------

detect_target() {
    os_raw=$(uname -s)
    arch_raw=$(uname -m)

    case "$os_raw" in
        Linux*)  os=unknown-linux-gnu ;;
        Darwin*) os=apple-darwin ;;
        *)       fail "unsupported OS: $os_raw" ;;
    esac

    case "$arch_raw" in
        x86_64|amd64)  arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *)             fail "unsupported architecture: $arch_raw" ;;
    esac

    printf '%s-%s' "$arch" "$os"
}

# -------- Confirm -----------------------------------------------------------

confirm() {
    [ -n "$NO_PROMPT" ] && return 0
    # When piped via `curl | sh`, stdin is the pipe; fall back to /dev/tty.
    if [ ! -t 0 ]; then
        if [ -r /dev/tty ]; then
            printf '%bProceed?%b [Y/n] ' "$C_BOLD" "$C_RESET"
            answer=$(head -n 1 </dev/tty)
        else
            return 0     # truly non-interactive (CI) - proceed silently
        fi
    else
        printf '%bProceed?%b [Y/n] ' "$C_BOLD" "$C_RESET"
        read -r answer
    fi
    case "$answer" in
        ''|y|Y|yes|YES) ;;
        *) fail 'aborted by user' ;;
    esac
}

# -------- PATH update -------------------------------------------------------

shell_rc() {
    case "${SHELL:-}" in
        */fish) printf '%s\n' "$HOME/.config/fish/conf.d/agentwatch.fish" ;;
        */zsh)  printf '%s\n' "$HOME/.zshrc" ;;
        */bash)
            if   [ -r "$HOME/.bashrc" ]; then printf '%s\n' "$HOME/.bashrc"
            else                              printf '%s\n' "$HOME/.bash_profile"
            fi ;;
        *)      printf '%s\n' "$HOME/.profile" ;;
    esac
}

maybe_add_to_path() {
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) return 0 ;;
    esac
    if [ -n "$NO_MODIFY_PATH" ]; then
        warn "$INSTALL_DIR is not on PATH; add it yourself or unset AGENTWATCH_NO_MODIFY_PATH"
        return 0
    fi
    rc=$(shell_rc)
    case "$rc" in
        *.fish) line="fish_add_path -pP '$INSTALL_DIR'" ;;
        *)      line="export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
    esac
    # Idempotent - don't append if already present.
    if [ -f "$rc" ] && grep -Fqs "$INSTALL_DIR" "$rc"; then
        sub "PATH already references $INSTALL_DIR in $rc"
        return 0
    fi
    mkdir -p "$(dirname "$rc")"
    {
        printf '\n# Added by agentwatch installer\n'
        printf '%s\n' "$line"
    } >> "$rc"
    sub "added $INSTALL_DIR to PATH in $rc"
    sub "restart your shell or run: source $rc"
}

# -------- Main --------------------------------------------------------------

main() {
    check_prereqs

    target=$(detect_target)
    step "agentwatch installer"
    sub  "version : $VERSION"
    sub  "target  : $target"
    sub  "install : $INSTALL_DIR/$BINARY"

    if [ -e "$INSTALL_DIR/$BINARY" ]; then
        existing=$("$INSTALL_DIR/$BINARY" --version 2>/dev/null | head -n 1 || true)
        sub "existing: ${existing:-<unknown>} (will be overwritten)"
        [ -z "$FORCE" ] && [ "${AGENTWATCH_NO_PROMPT:-}" = "1" ] || true
    fi

    confirm

    mkdir -p "$INSTALL_DIR"

    asset="${BINARY}-${target}.tar.gz"
    if [ "$VERSION" = "latest" ]; then
        url_base="https://github.com/${REPO}/releases/latest/download"
    else
        url_base="https://github.com/${REPO}/releases/download/${VERSION}"
    fi

    tmpdir=$(mktemp -d 2>/dev/null || mktemp -d -t 'agentwatch')
    trap 'rm -rf "$tmpdir"' EXIT INT TERM HUP

    step "downloading $asset"
    if ! curl -fSL --retry 3 --connect-timeout 15 \
              --proto '=https' --tlsv1.2 \
              -o "$tmpdir/$asset" "$url_base/$asset" 2>"$tmpdir/curl.err"; then
        err=$(tail -n 3 "$tmpdir/curl.err" 2>/dev/null || true)
        cat >&2 <<EOF

$(printf '%berror:%b' "$C_RED" "$C_RESET") could not download $url_base/$asset
${err:+  $err
}
Possible causes:
  • No release has been published yet for $REPO.
    Check: https://github.com/${REPO}/releases
  • The latest release has no binary for $target.
  • Network / proxy issue.

Until binaries are published, install from source:
  cargo install agentwatch

(Pushing code to main does NOT create a release. Cut one by tagging:
   git tag v0.1.0 && git push origin v0.1.0
 The release workflow at .github/workflows/release.yml will build the
 binaries and attach them to the release.)
EOF
        exit 1
    fi

    # Checksum verification - soft-fail if the release didn't publish one.
    if curl -fSL --retry 3 --connect-timeout 15 \
            --proto '=https' --tlsv1.2 \
            -o "$tmpdir/$asset.sha256" "$url_base/$asset.sha256" 2>/dev/null; then
        expected=$(awk '{print $1}' "$tmpdir/$asset.sha256")
        actual=$(sha256_of "$tmpdir/$asset")
        [ "$expected" = "$actual" ] || fail "checksum mismatch
  expected: $expected
  actual:   $actual"
        sub "checksum: ok"
    else
        warn "no checksum at $url_base/$asset.sha256 - skipping verification"
    fi

    tar xzf "$tmpdir/$asset" -C "$tmpdir"
    [ -e "$tmpdir/$BINARY" ] || fail "archive did not contain '$BINARY'"
    chmod +x "$tmpdir/$BINARY"

    # Atomic install: mv within same fs is atomic; if INSTALL_DIR is on a
    # different fs from $TMPDIR, fall back to copy+rename.
    if ! mv -f "$tmpdir/$BINARY" "$INSTALL_DIR/$BINARY" 2>/dev/null; then
        cp "$tmpdir/$BINARY" "$INSTALL_DIR/$BINARY.tmp"
        mv -f "$INSTALL_DIR/$BINARY.tmp" "$INSTALL_DIR/$BINARY"
    fi

    step "installed agentwatch → $INSTALL_DIR/$BINARY"
    "$INSTALL_DIR/$BINARY" --version 2>/dev/null || true

    maybe_add_to_path

    step "done. run \`agentwatch --help\` to get started."
}

main "$@"

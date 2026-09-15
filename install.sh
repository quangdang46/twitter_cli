#!/usr/bin/env bash
# twr installer — https://github.com/quangdang46/twitter_cli
#
# Usage:
#   curl -fsSL "https://raw.githubusercontent.com/quangdang46/twitter_cli/main/install.sh?$(date +%s)" | bash
#   curl -fsSL ".../install.sh" | bash -s -- --easy-mode --verify
#   curl -fsSL ".../install.sh" | bash -s -- --version v0.1.0
set -euo pipefail
umask 022

# === Config ===
BINARY_NAME="twr"
OWNER="quangdang46"
REPO="twitter_cli"
DEST="${DEST:-$HOME/.local/bin}"
VERSION="${VERSION:-}"
QUIET=0; EASY=0; VERIFY=0; FROM_SOURCE=0; UNINSTALL=0
MAX_RETRIES=3; DOWNLOAD_TIMEOUT=120
LOCK_DIR="/tmp/${BINARY_NAME}-install.lock.d"
TMP=""

# === Logging ===
log_info()    { [ "$QUIET" -eq 1 ] && return; echo "[${BINARY_NAME}] $*" >&2; }
log_warn()    { echo "[${BINARY_NAME}] WARN: $*" >&2; }
log_success() { [ "$QUIET" -eq 1 ] && return; echo "✓ $*" >&2; }
die()         { echo "ERROR: $*" >&2; exit 1; }

usage() {
    cat <<EOF
twr installer

Usage: install.sh [options]

  --dest <dir>       Install directory (default: \$HOME/.local/bin)
  --system           Install to /usr/local/bin
  --version <vX.Y.Z> Pin a specific release (default: latest)
  --easy-mode        Auto-append PATH export to shell rc files
  --verify           Run '\$BIN --version' after install
  --from-source      Build with cargo instead of downloading a binary
  --uninstall        Remove the binary and any PATH lines this installer added
  --quiet, -q        Suppress non-error output
  -h, --help         Show this help
EOF
    exit 0
}

# === Cleanup & lock ===
cleanup() { rm -rf "$TMP" "$LOCK_DIR" 2>/dev/null || true; }
trap cleanup EXIT
acquire_lock() {
    mkdir "$LOCK_DIR" 2>/dev/null || die "Another install is running. If stuck: rm -rf $LOCK_DIR"
    echo $$ > "$LOCK_DIR/pid"
}

# === Args ===
while [ $# -gt 0 ]; do
    case "$1" in
        --dest)       DEST="$2";   shift 2;;
        --dest=*)     DEST="${1#*=}"; shift;;
        --version)    VERSION="$2"; shift 2;;
        --version=*)  VERSION="${1#*=}"; shift;;
        --system)     DEST="/usr/local/bin"; shift;;
        --easy-mode)  EASY=1;      shift;;
        --verify)     VERIFY=1;    shift;;
        --from-source) FROM_SOURCE=1; shift;;
        --quiet|-q)   QUIET=1;     shift;;
        --uninstall)  UNINSTALL=1; shift;;
        -h|--help)    usage;;
        *) log_warn "Unknown option: $1"; shift;;
    esac
done

# === Uninstall ===
if [ "$UNINSTALL" -eq 1 ]; then
    rm -f "$DEST/$BINARY_NAME"
    for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
        [ -f "$rc" ] && sed -i.bak "/${BINARY_NAME} installer/d" "$rc" 2>/dev/null && rm -f "${rc}.bak" || true
    done
    echo "✓ ${BINARY_NAME} uninstalled"
    exit 0
fi

# === Platform ===
detect_platform() {
    local os arch
    case "$(uname -s)" in
        Linux*)  os="linux" ;;
        Darwin*) os="darwin" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *) die "Unsupported OS: $(uname -s)" ;;
    esac
    case "$(uname -m)" in
        x86_64|amd64)   arch="x86_64" ;;
        aarch64|arm64)  arch="aarch64" ;;
        *) die "Unsupported arch: $(uname -m)" ;;
    esac
    echo "${os}_${arch}"
}

# Maps our platform string to the release asset suffix baked into release.yml.
platform_to_suffix() {
    case "$1" in
        linux_x86_64)   echo "linux-x86_64" ;;
        linux_aarch64)  echo "linux-aarch64" ;;
        darwin_x86_64)  echo "macos-x86_64" ;;
        darwin_aarch64) echo "macos-aarch64" ;;
        windows_x86_64) echo "windows-x86_64" ;;
        *) die "No release asset for platform: $1 — try --from-source" ;;
    esac
}

# === Version resolution ===
resolve_version() {
    [ -n "$VERSION" ] && return 0
    VERSION=$(curl -fsSL --connect-timeout 10 --max-time 30 \
        -H "Accept: application/vnd.github.v3+json" \
        "https://api.github.com/repos/${OWNER}/${REPO}/releases/latest" \
        2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/') || true
    if [ -z "$VERSION" ]; then
        VERSION=$(curl -fsSL -o /dev/null -w '%{url_effective}' \
            "https://github.com/${OWNER}/${REPO}/releases/latest" \
            2>/dev/null | sed -E 's|.*/tag/||') || true
    fi
    [[ "$VERSION" =~ ^v[0-9] ]] || die "Could not resolve the latest version — pass --version vX.Y.Z or --from-source"
    log_info "Latest release: $VERSION"
}

# === Download with retry + resume ===
download_file() {
    local url="$1" dest="$2" partial="${2}.part" attempt=0
    while [ $attempt -lt $MAX_RETRIES ]; do
        attempt=$((attempt + 1))
        curl -fL \
            --connect-timeout 30 \
            --max-time "$DOWNLOAD_TIMEOUT" \
            --retry 2 \
            $( [ -s "$partial" ] && echo "--continue-at -" ) \
            $( [ "$QUIET" -eq 0 ] && [ -t 2 ] && echo "--progress-bar" || echo "-sS" ) \
            -o "$partial" "$url" && mv -f "$partial" "$dest" && return 0
        [ $attempt -lt $MAX_RETRIES ] && { log_warn "Download failed, retrying in 3s..."; sleep 3; }
    done
    return 1
}

# === Atomic install ===
install_binary_atomic() {
    local src="$1" dest="$2" tmp="${2}.tmp.$$"
    install -m 0755 "$src" "$tmp" && mv -f "$tmp" "$dest" || { rm -f "$tmp"; die "Failed to install binary to $dest"; }
}

# Overwrite a stale copy already on PATH so `twr --version` doesn't keep
# resolving to an old build after a "successful" install.
override_stale_path_copy() {
    hash -r 2>/dev/null || true
    local existing_bin; existing_bin=$(command -v "$BINARY_NAME" 2>/dev/null || true)
    [ -z "$existing_bin" ] && return 0
    local existing_dir; existing_dir="$(cd "$(dirname "$existing_bin")" && pwd)"
    local dest_dir; dest_dir="$(cd "$DEST" && pwd)"
    [ "$existing_dir" = "$dest_dir" ] && return 0

    if cp -f "$DEST/$BINARY_NAME" "$existing_bin" 2>/dev/null; then
        log_success "replaced $existing_bin"
    elif command -v sudo >/dev/null 2>&1 && sudo -n cp -f "$DEST/$BINARY_NAME" "$existing_bin" 2>/dev/null; then
        log_success "replaced $existing_bin (via passwordless sudo)"
    elif command -v sudo >/dev/null 2>&1 && [ -t 0 ]; then
        log_info "need elevated permission to replace $existing_bin — you may be prompted for your password"
        sudo cp -f "$DEST/$BINARY_NAME" "$existing_bin" 2>/dev/null \
            && log_success "replaced $existing_bin (via sudo)" \
            || log_warn "could not update $existing_bin even with sudo — remove it manually"
    else
        log_warn "could not update $existing_bin — you may need sudo or remove it manually"
    fi

    hash -r 2>/dev/null || true
    local resolved_bin; resolved_bin=$(command -v "$BINARY_NAME" 2>/dev/null || true)
    local new_ver resolved_ver
    new_ver=$("$DEST/$BINARY_NAME" --version 2>/dev/null || true)
    if [ -n "$resolved_bin" ] && [ "$resolved_bin" != "$DEST/$BINARY_NAME" ]; then
        resolved_ver=$("$resolved_bin" --version 2>/dev/null || true)
        if [ "$resolved_ver" != "$new_ver" ]; then
            log_warn "===================================================="
            log_warn "  '$BINARY_NAME' on PATH still resolves to an OLDER copy!"
            log_warn "  PATH resolves to : $resolved_bin ($resolved_ver)"
            log_warn "  just installed    : $DEST/$BINARY_NAME ($new_ver)"
            log_warn "  Fix: remove the old copy or reorder PATH, then restart your shell."
            log_warn "===================================================="
        fi
    fi
}

# === PATH ===
maybe_add_path() {
    case ":$PATH:" in *":$DEST:"*) return 0;; esac
    if [ "$EASY" -eq 1 ]; then
        for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
            [ -f "$rc" ] && [ -w "$rc" ] || continue
            grep -qF "$DEST" "$rc" && continue
            printf '\nexport PATH="%s:$PATH"  # %s installer\n' "$DEST" "$BINARY_NAME" >> "$rc"
        done
        log_warn "PATH updated — restart your shell or run: export PATH=\"$DEST:\$PATH\""
    else
        log_warn "Add to PATH: export PATH=\"$DEST:\$PATH\" (or re-run with --easy-mode)"
    fi
}

# === From-source fallback ===
build_from_source() {
    command -v cargo >/dev/null || die "Rust/cargo not found. Install it: https://rustup.rs"
    log_info "Building from source (this may take a few minutes)..."
    git clone --depth 1 "https://github.com/${OWNER}/${REPO}.git" "$TMP/src"
    ( cd "$TMP/src" && CARGO_TARGET_DIR="$TMP/target" cargo build --release -p "$BINARY_NAME" )
    install_binary_atomic "$TMP/target/release/$BINARY_NAME" "$DEST/$BINARY_NAME"
}

main() {
    acquire_lock
    TMP=$(mktemp -d)
    mkdir -p "$DEST"

    local platform suffix
    platform=$(detect_platform)
    log_info "Platform: $platform | Dest: $DEST"

    if [ "$FROM_SOURCE" -eq 0 ]; then
        suffix=$(platform_to_suffix "$platform") || { build_from_source; suffix=""; }
        if [ -n "$suffix" ]; then
            resolve_version
            local ext="tar.gz"
            [[ "$suffix" == windows* ]] && ext="zip"
            local archive="${BINARY_NAME}-${suffix}.${ext}"
            local url="https://github.com/${OWNER}/${REPO}/releases/download/${VERSION}/${archive}"

            if download_file "$url" "$TMP/$archive"; then
                if download_file "${url}.sha256" "$TMP/${archive}.sha256" 2>/dev/null; then
                    local expected actual
                    expected=$(awk '{print $1}' "$TMP/${archive}.sha256")
                    actual=$(sha256sum "$TMP/$archive" 2>/dev/null | awk '{print $1}' \
                          || shasum -a 256 "$TMP/$archive" | awk '{print $1}')
                    [ "$expected" = "$actual" ] || die "Checksum mismatch for $archive — aborting"
                    log_info "Checksum verified"
                else
                    log_warn "No checksum sidecar found for $archive — skipping verification"
                fi
                case "$archive" in
                    *.tar.gz) tar -xzf "$TMP/$archive" -C "$TMP" ;;
                    *.zip)    unzip -q "$TMP/$archive" -d "$TMP" ;;
                esac
                local bin_path
                bin_path=$(find "$TMP" -maxdepth 2 -name "$BINARY_NAME" -type f -perm -111 2>/dev/null | head -1)
                [ -n "$bin_path" ] || die "Binary not found inside the downloaded archive"
                install_binary_atomic "$bin_path" "$DEST/$BINARY_NAME"
            else
                log_warn "Binary download failed — falling back to building from source"
                build_from_source
            fi
        fi
    else
        build_from_source
    fi

    override_stale_path_copy
    maybe_add_path

    if [ "$VERIFY" -eq 1 ]; then
        "$DEST/$BINARY_NAME" --version || die "Post-install verification failed"
    fi

    echo ""
    echo "✓ ${BINARY_NAME} installed -> $DEST/$BINARY_NAME"
    echo "  $("$DEST/$BINARY_NAME" --version 2>/dev/null || echo 'version check skipped')"
    echo ""
    echo "  Quick start:"
    echo "    $BINARY_NAME status --json"
    echo ""
    echo "  Note: twr is pre-implementation (see PLAN.md) — status/schema are"
    echo "  scaffold stubs today, not real network calls."
}

# curl | bash safety: buffer the whole script before executing, so a
# truncated download never runs a half-written main().
if [[ "${BASH_SOURCE[0]:-}" == "${0:-}" ]] || [[ -z "${BASH_SOURCE[0]:-}" ]]; then
    { main "$@"; }
fi

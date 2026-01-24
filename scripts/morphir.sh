#!/usr/bin/env bash
#
# morphir - Launcher script for the Morphir CLI
#
# This script automatically downloads and runs the correct version of morphir.
# It supports version pinning, multiple installation backends, and self-management.
#
# Usage:
#   morphir <args>              Run morphir with the resolved version
#   morphir +0.1.0 <args>       Run with a specific version
#   morphir self upgrade        Upgrade to latest version
#   morphir self list           List installed versions
#   morphir self which          Show which version would be used
#   morphir self install <ver>  Install a specific version
#   morphir self prune          Remove old versions
#   morphir self update         Update this launcher script
#
# Environment variables:
#   MORPHIR_VERSION   Override version to use
#   MORPHIR_BACKEND   Force backend: mise, binstall, github, cargo
#   MORPHIR_HOME      Override home directory (default: ~/.morphir)
#

set -euo pipefail

# Configuration
MORPHIR_HOME="${MORPHIR_HOME:-$HOME/.morphir}"
REPO="finos/morphir-rust"
GITHUB_API="https://api.github.com/repos/$REPO"
GITHUB_RELEASES="https://github.com/$REPO/releases"
CACHE_TTL=86400  # 24 hours in seconds

# Colors for output (disabled if not a terminal)
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    NC=''
fi

# Logging functions
log_info() { echo -e "${BLUE}info${NC}: $*" >&2; }
log_success() { echo -e "${GREEN}success${NC}: $*" >&2; }
log_warn() { echo -e "${YELLOW}warning${NC}: $*" >&2; }
log_error() { echo -e "${RED}error${NC}: $*" >&2; }

# Detect platform and architecture
detect_platform() {
    local os arch

    case "$(uname -s)" in
        Darwin) os="apple-darwin" ;;
        Linux)
            # Check if musl-based
            if ldd --version 2>&1 | grep -q musl; then
                os="unknown-linux-musl"
            else
                os="unknown-linux-gnu"
            fi
            ;;
        MINGW*|MSYS*|CYGWIN*) os="pc-windows-msvc" ;;
        *) log_error "Unsupported OS: $(uname -s)"; exit 1 ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64) arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) log_error "Unsupported architecture: $(uname -m)"; exit 1 ;;
    esac

    echo "${arch}-${os}"
}

# Find .morphir-version file by walking up directory tree
find_version_file() {
    local dir="$PWD"
    while [[ "$dir" != "/" ]]; do
        if [[ -f "$dir/.morphir-version" ]]; then
            echo "$dir/.morphir-version"
            return 0
        fi
        dir="$(dirname "$dir")"
    done
    return 1
}

# Find morphir.toml and extract version
find_toml_version() {
    local dir="$PWD"
    while [[ "$dir" != "/" ]]; do
        if [[ -f "$dir/morphir.toml" ]]; then
            # Try to extract version from [morphir] section or top-level
            # Supports: version = "0.1.0" or morphir-version = "0.1.0"
            local version
            version=$(grep -E '^\s*(morphir-)?version\s*=' "$dir/morphir.toml" | head -1 | sed 's/.*=\s*["'"'"']\([^"'"'"']*\)["'"'"'].*/\1/')
            if [[ -n "$version" ]]; then
                echo "$version"
                return 0
            fi
        fi
        dir="$(dirname "$dir")"
    done
    return 1
}

# Get latest version from GitHub API (with caching)
get_latest_version() {
    local cache_file="$MORPHIR_HOME/.latest"
    local cache_time_file="$MORPHIR_HOME/.latest-time"
    local now
    now=$(date +%s)

    # Check cache
    if [[ -f "$cache_file" && -f "$cache_time_file" ]]; then
        local cached_time
        cached_time=$(cat "$cache_time_file")
        if (( now - cached_time < CACHE_TTL )); then
            cat "$cache_file"
            return 0
        fi
    fi

    # Fetch from GitHub
    local version
    if command -v curl &>/dev/null; then
        version=$(curl -fsSL "$GITHUB_API/releases/latest" | grep -o '"tag_name": *"[^"]*"' | head -1 | cut -d'"' -f4)
    elif command -v wget &>/dev/null; then
        version=$(wget -qO- "$GITHUB_API/releases/latest" | grep -o '"tag_name": *"[^"]*"' | head -1 | cut -d'"' -f4)
    else
        log_error "Neither curl nor wget found"
        exit 1
    fi

    # Strip leading 'v' if present
    version="${version#v}"

    # Cache the result
    mkdir -p "$MORPHIR_HOME"
    echo "$version" > "$cache_file"
    echo "$now" > "$cache_time_file"

    echo "$version"
}

# Resolve which version to use
resolve_version() {
    local version=""
    local source=""

    # 1. Check for +version argument (handled by caller, passed as $1)
    if [[ -n "${1:-}" ]]; then
        version="$1"
        source="+argument"
    # 2. Check MORPHIR_VERSION env var
    elif [[ -n "${MORPHIR_VERSION:-}" ]]; then
        version="$MORPHIR_VERSION"
        source="MORPHIR_VERSION env"
    # 3. Check .morphir-version file
    elif version_file=$(find_version_file); then
        version=$(cat "$version_file" | tr -d '[:space:]')
        source=".morphir-version"
    # 4. Check morphir.toml file
    elif version=$(find_toml_version); then
        source="morphir.toml"
    # 5. Get latest from GitHub
    else
        version=$(get_latest_version)
        source="latest"
    fi

    # Strip leading 'v' if present
    version="${version#v}"

    echo "$version"
}

# Check if a version is installed
is_installed() {
    local version="$1"
    [[ -x "$MORPHIR_HOME/versions/$version/morphir-bin" ]]
}

# Get the binary path for a version
get_binary_path() {
    local version="$1"
    echo "$MORPHIR_HOME/versions/$version/morphir-bin"
}

# Check if a command exists
has_command() {
    command -v "$1" &>/dev/null
}

# Detect which backend to use
detect_backend() {
    local forced="${MORPHIR_BACKEND:-}"

    if [[ -n "$forced" ]]; then
        echo "$forced"
        return
    fi

    # Priority order: mise > binstall > github
    if has_command mise; then
        echo "mise"
    elif has_command cargo-binstall; then
        echo "binstall"
    else
        echo "github"
    fi
}

# Install using mise
install_mise() {
    local version="$1"
    log_info "Installing morphir $version using mise..."

    # Use mise to install and link
    mise install "github:$REPO@v$version"

    # Copy binary to our versions directory
    local mise_path
    mise_path=$(mise where "github:$REPO@v$version")/morphir

    mkdir -p "$MORPHIR_HOME/versions/$version"
    cp "$mise_path" "$MORPHIR_HOME/versions/$version/morphir-bin"
    chmod +x "$MORPHIR_HOME/versions/$version/morphir-bin"
}

# Install using cargo-binstall
install_binstall() {
    local version="$1"
    log_info "Installing morphir $version using cargo-binstall..."

    local temp_dir
    temp_dir=$(mktemp -d)
    trap "rm -rf '$temp_dir'" EXIT

    # Install to temp directory
    CARGO_HOME="$temp_dir" cargo binstall \
        --git "https://github.com/$REPO" \
        --tag "v$version" \
        --no-confirm \
        --root "$temp_dir" \
        morphir

    # Move to our versions directory
    mkdir -p "$MORPHIR_HOME/versions/$version"
    mv "$temp_dir/bin/morphir" "$MORPHIR_HOME/versions/$version/morphir-bin"
    chmod +x "$MORPHIR_HOME/versions/$version/morphir-bin"

    trap - EXIT
    rm -rf "$temp_dir"
}

# Install by downloading from GitHub releases
install_github() {
    local version="$1"
    local platform
    platform=$(detect_platform)

    log_info "Installing morphir $version from GitHub releases..."

    local archive_name="morphir-$version-$platform"
    local archive_ext="tgz"
    local url="$GITHUB_RELEASES/download/v$version/$archive_name.$archive_ext"

    # Windows uses zip
    if [[ "$platform" == *"windows"* ]]; then
        archive_ext="zip"
        url="$GITHUB_RELEASES/download/v$version/$archive_name.$archive_ext"
    fi

    local temp_dir
    temp_dir=$(mktemp -d)
    trap "rm -rf '$temp_dir'" EXIT

    # Download
    log_info "Downloading $url"
    if has_command curl; then
        curl -fsSL "$url" -o "$temp_dir/archive.$archive_ext"
    elif has_command wget; then
        wget -q "$url" -O "$temp_dir/archive.$archive_ext"
    else
        log_error "Neither curl nor wget found"
        exit 1
    fi

    # Extract
    cd "$temp_dir"
    if [[ "$archive_ext" == "zip" ]]; then
        unzip -q "archive.$archive_ext"
    else
        tar xzf "archive.$archive_ext"
    fi

    # Find and move binary
    mkdir -p "$MORPHIR_HOME/versions/$version"
    if [[ -f "morphir" ]]; then
        mv "morphir" "$MORPHIR_HOME/versions/$version/morphir-bin"
    elif [[ -f "morphir.exe" ]]; then
        mv "morphir.exe" "$MORPHIR_HOME/versions/$version/morphir-bin"
    else
        log_error "Could not find morphir binary in archive"
        exit 1
    fi
    chmod +x "$MORPHIR_HOME/versions/$version/morphir-bin"

    trap - EXIT
    rm -rf "$temp_dir"

    log_success "Installed morphir $version"
}

# Install using cargo install (compile from source)
install_cargo() {
    local version="$1"
    log_info "Installing morphir $version using cargo (compiling from source)..."
    log_warn "This may take a few minutes..."

    local temp_dir
    temp_dir=$(mktemp -d)
    trap "rm -rf '$temp_dir'" EXIT

    CARGO_HOME="$temp_dir" cargo install \
        --git "https://github.com/$REPO" \
        --tag "v$version" \
        --root "$temp_dir" \
        morphir

    mkdir -p "$MORPHIR_HOME/versions/$version"
    mv "$temp_dir/bin/morphir" "$MORPHIR_HOME/versions/$version/morphir-bin"
    chmod +x "$MORPHIR_HOME/versions/$version/morphir-bin"

    trap - EXIT
    rm -rf "$temp_dir"

    log_success "Installed morphir $version"
}

# Install a version using the appropriate backend
install_version() {
    local version="$1"
    local backend
    backend=$(detect_backend)

    case "$backend" in
        mise) install_mise "$version" ;;
        binstall) install_binstall "$version" ;;
        github) install_github "$version" ;;
        cargo) install_cargo "$version" ;;
        *)
            log_error "Unknown backend: $backend"
            exit 1
            ;;
    esac
}

# Ensure a version is installed
ensure_installed() {
    local version="$1"

    if ! is_installed "$version"; then
        install_version "$version"
    fi
}

# Run morphir with the given version and arguments
run_morphir() {
    local version="$1"
    shift

    ensure_installed "$version"

    local binary
    binary=$(get_binary_path "$version")

    exec "$binary" "$@"
}

# Handle 'self' subcommands
handle_self() {
    local cmd="${1:-help}"
    shift || true

    case "$cmd" in
        upgrade)
            log_info "Checking for updates..."
            local latest
            # Clear cache to force fresh check
            rm -f "$MORPHIR_HOME/.latest" "$MORPHIR_HOME/.latest-time"
            latest=$(get_latest_version)
            log_info "Latest version: $latest"

            if is_installed "$latest"; then
                log_info "Version $latest is already installed"
            else
                install_version "$latest"
            fi
            ;;

        list)
            echo "Installed versions:"
            if [[ -d "$MORPHIR_HOME/versions" ]]; then
                for dir in "$MORPHIR_HOME/versions"/*/; do
                    if [[ -d "$dir" ]]; then
                        local v
                        v=$(basename "$dir")
                        echo "  $v"
                    fi
                done
            else
                echo "  (none)"
            fi
            ;;

        which)
            local version
            version=$(resolve_version "")
            local binary
            binary=$(get_binary_path "$version")

            echo "Version: $version"
            if is_installed "$version"; then
                echo "Binary: $binary"
                echo "Status: installed"
            else
                echo "Binary: $binary (not installed)"
                echo "Status: will download on first use"
            fi
            echo "Backend: $(detect_backend)"
            ;;

        install)
            local version="${1:-}"
            if [[ -z "$version" ]]; then
                log_error "Usage: morphir self install <version>"
                exit 1
            fi
            version="${version#v}"
            install_version "$version"
            ;;

        prune)
            local current
            current=$(resolve_version "")
            log_info "Current version: $current (keeping)"

            if [[ -d "$MORPHIR_HOME/versions" ]]; then
                for dir in "$MORPHIR_HOME/versions"/*/; do
                    if [[ -d "$dir" ]]; then
                        local v
                        v=$(basename "$dir")
                        if [[ "$v" != "$current" ]]; then
                            log_info "Removing $v..."
                            rm -rf "$dir"
                        fi
                    fi
                done
            fi
            log_success "Pruned old versions"
            ;;

        update)
            log_info "Updating morphir launcher script..."
            local script_url="https://raw.githubusercontent.com/$REPO/main/scripts/morphir.sh"
            local script_path="${BASH_SOURCE[0]}"

            if has_command curl; then
                curl -fsSL "$script_url" -o "$script_path.new"
            elif has_command wget; then
                wget -q "$script_url" -O "$script_path.new"
            else
                log_error "Neither curl nor wget found"
                exit 1
            fi

            mv "$script_path.new" "$script_path"
            chmod +x "$script_path"
            log_success "Updated launcher script"
            ;;

        help|*)
            cat <<EOF
morphir self - Manage the morphir installation

Commands:
  upgrade          Download and install the latest version
  list             List installed versions
  which            Show which version would be used
  install <ver>    Install a specific version
  prune            Remove old versions (keeps current)
  update           Update this launcher script

Environment variables:
  MORPHIR_VERSION  Override version to use
  MORPHIR_BACKEND  Force backend: mise, binstall, github, cargo
  MORPHIR_HOME     Override home directory (default: ~/.morphir)
EOF
            ;;
    esac
}

# Main entry point
main() {
    # Create home directory if needed
    mkdir -p "$MORPHIR_HOME"

    # Parse arguments
    local version_override=""
    local args=()

    while [[ $# -gt 0 ]]; do
        case "$1" in
            +*)
                # Version override: +0.1.0
                version_override="${1#+}"
                version_override="${version_override#v}"
                shift
                ;;
            self)
                # Handle self subcommand
                shift
                handle_self "$@"
                exit 0
                ;;
            *)
                args+=("$1")
                shift
                ;;
        esac
    done

    # Resolve version
    local version
    version=$(resolve_version "$version_override")

    # Run morphir
    run_morphir "$version" "${args[@]}"
}

main "$@"

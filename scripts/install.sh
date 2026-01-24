#!/usr/bin/env bash
#
# Morphir CLI Installer
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/finos/morphir-rust/main/scripts/install.sh | bash
#
# Options (pass via environment or arguments):
#   MORPHIR_HOME    Installation directory (default: ~/.morphir)
#   --no-modify-path  Don't modify shell profile
#   --version <ver>   Install specific version
#

set -euo pipefail

# Configuration
MORPHIR_HOME="${MORPHIR_HOME:-$HOME/.morphir}"
REPO="finos/morphir-rust"
SCRIPT_URL="https://raw.githubusercontent.com/$REPO/main/scripts/morphir.sh"

# Colors
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    BOLD=''
    NC=''
fi

log_info() { echo -e "${BLUE}info${NC}: $*"; }
log_success() { echo -e "${GREEN}success${NC}: $*"; }
log_warn() { echo -e "${YELLOW}warning${NC}: $*"; }
log_error() { echo -e "${RED}error${NC}: $*"; }

# Parse arguments
MODIFY_PATH=true
VERSION=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-modify-path)
            MODIFY_PATH=false
            shift
            ;;
        --version)
            VERSION="$2"
            shift 2
            ;;
        *)
            log_error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Check for curl or wget
download() {
    local url="$1"
    local dest="$2"

    if command -v curl &>/dev/null; then
        curl -fsSL "$url" -o "$dest"
    elif command -v wget &>/dev/null; then
        wget -q "$url" -O "$dest"
    else
        log_error "Neither curl nor wget found. Please install one and try again."
        exit 1
    fi
}

# Detect shell and profile file
detect_shell_profile() {
    local shell_name
    shell_name=$(basename "${SHELL:-/bin/bash}")

    case "$shell_name" in
        bash)
            if [[ -f "$HOME/.bashrc" ]]; then
                echo "$HOME/.bashrc"
            elif [[ -f "$HOME/.bash_profile" ]]; then
                echo "$HOME/.bash_profile"
            else
                echo "$HOME/.bashrc"
            fi
            ;;
        zsh)
            echo "${ZDOTDIR:-$HOME}/.zshrc"
            ;;
        fish)
            echo "$HOME/.config/fish/config.fish"
            ;;
        *)
            echo "$HOME/.profile"
            ;;
    esac
}

# Check if path is already in profile
path_in_profile() {
    local profile="$1"
    local path_entry="$2"

    if [[ -f "$profile" ]]; then
        grep -q "$path_entry" "$profile" 2>/dev/null
    else
        return 1
    fi
}

# Add path to shell profile
add_to_path() {
    local profile
    profile=$(detect_shell_profile)
    local bin_dir="$MORPHIR_HOME/bin"
    local shell_name
    shell_name=$(basename "${SHELL:-/bin/bash}")

    if path_in_profile "$profile" "$bin_dir"; then
        log_info "PATH already contains $bin_dir"
        return 0
    fi

    local path_line
    case "$shell_name" in
        fish)
            path_line="set -gx PATH \"$bin_dir\" \$PATH"
            ;;
        *)
            path_line="export PATH=\"$bin_dir:\$PATH\""
            ;;
    esac

    echo "" >> "$profile"
    echo "# Added by morphir installer" >> "$profile"
    echo "$path_line" >> "$profile"

    log_success "Added $bin_dir to PATH in $profile"
    log_info "Run 'source $profile' or restart your shell to use morphir"
}

# Main installation
main() {
    echo -e "${BOLD}Morphir CLI Installer${NC}"
    echo ""

    # Create directories
    log_info "Creating $MORPHIR_HOME/bin..."
    mkdir -p "$MORPHIR_HOME/bin"

    # Download launcher script
    log_info "Downloading morphir launcher..."
    download "$SCRIPT_URL" "$MORPHIR_HOME/bin/morphir"
    chmod +x "$MORPHIR_HOME/bin/morphir"

    # Add to PATH
    if [[ "$MODIFY_PATH" == true ]]; then
        if [[ ":$PATH:" != *":$MORPHIR_HOME/bin:"* ]]; then
            add_to_path
        else
            log_info "$MORPHIR_HOME/bin is already in PATH"
        fi
    fi

    # Pre-install a version if specified
    if [[ -n "$VERSION" ]]; then
        log_info "Pre-installing morphir $VERSION..."
        "$MORPHIR_HOME/bin/morphir" self install "$VERSION"
    fi

    echo ""
    log_success "Morphir installed successfully!"
    echo ""
    echo -e "To get started, run:"
    echo ""

    if [[ ":$PATH:" != *":$MORPHIR_HOME/bin:"* ]]; then
        local profile
        profile=$(detect_shell_profile)
        echo -e "  ${BLUE}source $profile${NC}  # or restart your shell"
    fi

    echo -e "  ${BLUE}morphir --help${NC}"
    echo ""
    echo -e "The first time you run a morphir command, it will automatically"
    echo -e "download the latest version."
    echo ""
    echo -e "Useful commands:"
    echo -e "  ${BLUE}morphir self upgrade${NC}    # Update to latest version"
    echo -e "  ${BLUE}morphir self list${NC}       # List installed versions"
    echo -e "  ${BLUE}morphir self which${NC}      # Show current version"
    echo ""
}

main "$@"

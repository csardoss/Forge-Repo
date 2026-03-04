#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
# Forge CLI Installer
#
# Interactive installer that downloads Forge via device pairing.
# No API key required — just approve the pairing request in your browser.
#
# Usage:
#   curl -sSL https://raw.githubusercontent.com/csardoss/Forge-Repo/main/install.sh | bash
#   # or
#   ./install.sh
#
# Environment overrides:
#   FORGE_PORTAL_URL   Portal base URL (default: https://artifacts.digitalsecurityguard.com)
#   FORGE_ORG          Organization slug (prompted if not set)
#   FORGE_INSTALL_DIR  Install directory (default: /usr/local/bin)
#   FORGE_TOKEN        Skip pairing if you already have a bearer token
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────
PORTAL_URL="${FORGE_PORTAL_URL:-https://artifacts.digitalsecurityguard.com}"
ORG_SLUG="${FORGE_ORG:-}"
INSTALL_DIR="${FORGE_INSTALL_DIR:-/usr/local/bin}"
TOKEN="${FORGE_TOKEN:-}"
PLATFORM_ARCH=""
PROJECT="internal-tools"
TOOL="forge"
LATEST_FILENAME="forge"

# ── Colors ────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

info()  { echo -e "${CYAN}${BOLD}=>${RESET} $1"; }
ok()    { echo -e "${GREEN}${BOLD}✓${RESET}  $1"; }
warn()  { echo -e "${YELLOW}${BOLD}⚠${RESET}  $1"; }
err()   { echo -e "${RED}${BOLD}✗${RESET}  $1" >&2; }
die()   { err "$1"; exit 1; }

# ── Dependency checks ────────────────────────────────────────────────
check_deps() {
    for cmd in curl jq sha256sum; do
        if ! command -v "$cmd" &>/dev/null; then
            die "Required command '$cmd' not found. Install it and retry."
        fi
    done
}

# ── Platform detection ───────────────────────────────────────────────
detect_platform() {
    local os arch
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    arch="$(uname -m)"

    case "$os" in
        linux)  os="linux" ;;
        darwin) os="darwin" ;;
        *)      die "Unsupported OS: $os" ;;
    esac

    case "$arch" in
        x86_64|amd64)   arch="amd64" ;;
        aarch64|arm64)  arch="arm64" ;;
        armv7l)         arch="armv7" ;;
        *)              die "Unsupported architecture: $arch" ;;
    esac

    PLATFORM_ARCH="${os}-${arch}"
}

# ── Device pairing flow ─────────────────────────────────────────────
do_pairing() {
    local instance_id
    instance_id="$(hostname 2>/dev/null || echo 'unknown')"

    info "Starting device pairing with ${BOLD}${PORTAL_URL}${RESET}"
    echo ""

    # Start pairing
    local start_resp
    start_resp=$(curl -sS -X POST "${PORTAL_URL}/api/v2/pairing/start" \
        -H "Content-Type: application/json" \
        -d "{
            \"org_slug\": \"${ORG_SLUG}\",
            \"app_id\": \"forge-installer\",
            \"instance_id\": \"${instance_id}\",
            \"requested_scopes\": [\"registry:read\", \"download\", \"manifest:read\", \"latest:read\"],
            \"metadata\": {
                \"hostname\": \"${instance_id}\",
                \"platform\": \"$(uname -s | tr '[:upper:]' '[:lower:]')\",
                \"arch\": \"$(uname -m)\"
            }
        }" 2>/dev/null) || die "Failed to connect to portal. Check your network and portal URL."

    # Check for error
    if echo "$start_resp" | jq -e '.detail' &>/dev/null; then
        die "Pairing failed: $(echo "$start_resp" | jq -r '.detail')"
    fi

    local pairing_code pairing_url
    pairing_code=$(echo "$start_resp" | jq -r '.pairing_code')
    pairing_url=$(echo "$start_resp" | jq -r '.pairing_url')

    echo -e "  ┌─────────────────────────────────────────────────────────┐"
    echo -e "  │                                                         │"
    echo -e "  │   Pairing code:  ${BOLD}${CYAN}${pairing_code}${RESET}                           │"
    echo -e "  │                                                         │"
    echo -e "  │   Approve at:    ${PORTAL_URL}${pairing_url}  │"
    echo -e "  │                                                         │"
    echo -e "  │   Open the URL above in your browser and click          │"
    echo -e "  │   ${BOLD}Approve${RESET} to authorize this installation.              │"
    echo -e "  │                                                         │"
    echo -e "  └─────────────────────────────────────────────────────────┘"
    echo ""

    # Poll for approval
    info "Waiting for approval..."
    local status_resp status exchange_token
    while true; do
        sleep 2
        status_resp=$(curl -sS "${PORTAL_URL}/api/v2/pairing/status/${pairing_code}" 2>/dev/null) || continue
        status=$(echo "$status_resp" | jq -r '.status')

        case "$status" in
            approved)
                exchange_token=$(echo "$status_resp" | jq -r '.exchange_token')
                break
                ;;
            denied)
                echo ""
                die "Pairing was denied."
                ;;
            expired)
                echo ""
                die "Pairing code expired. Run the installer again."
                ;;
            pending)
                ;;
            *)
                echo ""
                die "Unexpected pairing status: $status"
                ;;
        esac
    done

    ok "Pairing approved!"
    echo ""

    # Exchange for token (must happen within 60 seconds of approval)
    info "Exchanging pairing code for access token..."
    local exchange_resp
    exchange_resp=$(curl -sS -X POST "${PORTAL_URL}/api/v2/pairing/exchange" \
        -H "Content-Type: application/json" \
        -d "{
            \"pairing_code\": \"${pairing_code}\",
            \"exchange_token\": \"${exchange_token}\"
        }" 2>/dev/null) || die "Token exchange failed."

    if echo "$exchange_resp" | jq -e '.detail' &>/dev/null; then
        die "Exchange failed: $(echo "$exchange_resp" | jq -r '.detail')"
    fi

    TOKEN=$(echo "$exchange_resp" | jq -r '.access_token')
    local expires_at
    expires_at=$(echo "$exchange_resp" | jq -r '.expires_at')

    ok "Authenticated (token expires: ${expires_at})"
    echo ""
}

# ── Download and install ─────────────────────────────────────────────
do_install() {
    info "Requesting download URL for ${BOLD}${PROJECT}/${TOOL}${RESET} (${PLATFORM_ARCH})..."

    local presign_resp
    presign_resp=$(curl -sS -X POST "${PORTAL_URL}/api/v2/presign-latest" \
        -H "Authorization: Bearer ${TOKEN}" \
        -H "Content-Type: application/json" \
        -d "{
            \"project\": \"${PROJECT}\",
            \"tool\": \"${TOOL}\",
            \"platform_arch\": \"${PLATFORM_ARCH}\",
            \"latest_filename\": \"${LATEST_FILENAME}\"
        }" 2>/dev/null) || die "Failed to get download URL."

    if echo "$presign_resp" | jq -e '.detail' &>/dev/null; then
        die "Download failed: $(echo "$presign_resp" | jq -r '.detail')"
    fi

    local url sha256 size_bytes filename
    url=$(echo "$presign_resp" | jq -r '.url')
    sha256=$(echo "$presign_resp" | jq -r '.sha256 // empty')
    size_bytes=$(echo "$presign_resp" | jq -r '.size_bytes // empty')
    filename=$(echo "$presign_resp" | jq -r '.filename')

    local size_display=""
    if [ -n "$size_bytes" ] && [ "$size_bytes" != "null" ]; then
        size_display=" ($(numfmt --to=iec-i --suffix=B "$size_bytes" 2>/dev/null || echo "${size_bytes} bytes"))"
    fi

    info "Downloading ${BOLD}${filename}${RESET}${size_display}..."

    local tmp_file="${INSTALL_DIR}/.forge.install.tmp"
    curl -sS -L -o "$tmp_file" "$url" || die "Download failed."

    # Verify SHA-256
    if [ -n "$sha256" ] && [ "$sha256" != "null" ]; then
        info "Verifying SHA-256 checksum..."
        local actual_sha256
        actual_sha256=$(sha256sum "$tmp_file" | awk '{print $1}')
        if [ "$actual_sha256" != "$sha256" ]; then
            rm -f "$tmp_file"
            die "SHA-256 mismatch!\n  Expected: ${sha256}\n  Got:      ${actual_sha256}"
        fi
        ok "Checksum verified"
    else
        warn "No checksum available — skipping verification"
    fi

    # Install
    chmod 755 "$tmp_file"
    mv "$tmp_file" "${INSTALL_DIR}/forge"

    ok "Installed to ${BOLD}${INSTALL_DIR}/forge${RESET}"
}

# ── Save credentials for forge CLI ───────────────────────────────────
save_credentials() {
    local config_dir="${HOME}/.config/forge"
    mkdir -p "$config_dir"
    chmod 700 "$config_dir"

    cat > "${config_dir}/credentials.json.tmp" <<EOF
{
  "access_token": "${TOKEN}",
  "expires_at": null,
  "scopes": ["registry:read", "download", "manifest:read", "latest:read"],
  "portal_url": "${PORTAL_URL}",
  "org_slug": "${ORG_SLUG}"
}
EOF
    chmod 600 "${config_dir}/credentials.json.tmp"
    mv "${config_dir}/credentials.json.tmp" "${config_dir}/credentials.json"

    ok "Credentials saved to ~/.config/forge/credentials.json"
}

# ── Main ─────────────────────────────────────────────────────────────
main() {
    echo ""
    echo -e "${BOLD}  Forge CLI Installer${RESET}"
    echo -e "  ────────────────────"
    echo ""

    check_deps
    detect_platform
    ok "Platform: ${PLATFORM_ARCH}"

    # Ensure install directory exists and is writable
    if [ ! -d "$INSTALL_DIR" ]; then
        info "Creating install directory: ${INSTALL_DIR}"
        mkdir -p "$INSTALL_DIR" || die "Cannot create ${INSTALL_DIR}. Try: sudo mkdir -p ${INSTALL_DIR} && sudo chown \$(whoami) ${INSTALL_DIR}"
    fi
    if [ ! -w "$INSTALL_DIR" ]; then
        die "${INSTALL_DIR} is not writable. Try: sudo chown \$(whoami) ${INSTALL_DIR}"
    fi

    # Prompt for org slug if not set
    if [ -z "$ORG_SLUG" ]; then
        echo ""
        read -rp "  Organization slug: " ORG_SLUG
        if [ -z "$ORG_SLUG" ]; then
            die "Organization slug is required."
        fi
    fi

    echo ""

    # Authenticate
    if [ -z "$TOKEN" ]; then
        do_pairing
    else
        ok "Using existing token from FORGE_TOKEN"
        echo ""
    fi

    # Download and install
    do_install
    echo ""

    # Save credentials so `forge catalog`, `forge install`, etc. work immediately
    save_credentials
    echo ""

    # Verify installation
    if "${INSTALL_DIR}/forge" --version &>/dev/null; then
        local version
        version=$("${INSTALL_DIR}/forge" --version)
        echo ""
        echo -e "  ${GREEN}${BOLD}Installation complete!${RESET}"
        echo ""
        echo -e "  ${version}"
        echo ""
        echo -e "  Get started:"
        echo -e "    forge catalog          # Browse available tools"
        echo -e "    forge install <tool>    # Install a tool"
        echo -e "    forge list             # List installed tools"
        echo -e "    forge --help           # Full command reference"
        echo ""
    else
        warn "forge binary installed but could not verify execution."
        warn "You may need to add ${INSTALL_DIR} to your PATH."
    fi
}

main "$@"

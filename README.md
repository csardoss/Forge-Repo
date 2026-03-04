# Forge CLI

A tool manager for the [Artifact Upload Portal](https://artifacts.digitalsecurityguard.com). Install, upgrade, verify, and manage binary tools distributed through the portal — all authenticated via device pairing (no API keys to manage).

## Quick Install

```bash
curl -sSL https://raw.githubusercontent.com/csardoss/Forge-Repo/main/install.sh | bash
```

The installer will:
1. Detect your platform (linux-amd64, linux-arm64, etc.)
2. Start a device pairing session — you'll see a code and approval URL
3. Open the URL in your browser and click **Approve**
4. Download, verify (SHA-256), and install the `forge` binary to `/usr/local/bin/`
5. Save credentials so all `forge` commands work immediately

### Requirements

- `curl`, `jq`, `sha256sum` (pre-installed on most Linux systems)
- Write access to `/usr/local/bin/` (or set `FORGE_INSTALL_DIR`)
- A user account on the Artifact Portal with org membership

### Custom Install Directory

```bash
FORGE_INSTALL_DIR=/opt/tools curl -sSL https://raw.githubusercontent.com/csardoss/Forge-Repo/main/install.sh | bash
```

### Non-Interactive Install (CI/CD)

If you already have an API token:

```bash
FORGE_TOKEN=apt_xxxxx FORGE_ORG=my-org curl -sSL https://raw.githubusercontent.com/csardoss/Forge-Repo/main/install.sh | bash
```

## Commands

| Command | Description |
|---------|-------------|
| `forge login` | Authenticate via device pairing |
| `forge catalog` | Browse available tools in the registry |
| `forge install <tool>` | Install a tool (resolves dependencies automatically) |
| `forge upgrade [<tool> \| --all]` | Upgrade installed tools to latest version |
| `forge uninstall <tool>` | Remove an installed tool |
| `forge list` | List installed tools |
| `forge verify [<tool> \| --all]` | Verify SHA-256 integrity of installed tools |
| `forge info <tool>` | Show detailed tool information |

## Usage Examples

### First-Time Setup

```bash
# Install forge (if not using the curl one-liner)
forge login

# Browse what's available
forge catalog
```

### Install a Tool

```bash
# Auto-resolves project if tool name is unique
forge install mytool

# Specify project if ambiguous
forge install mytool --project security-tools

# Install to a custom directory
forge install mytool --path /opt/tools

# Skip all prompts (for automation)
forge install mytool --yes

# Include optional dependencies
forge install mytool --with-optional
```

### Manage Installed Tools

```bash
# See what's installed
forge list

# Verify nothing has been tampered with
forge verify --all

# Upgrade everything
forge upgrade --all

# Upgrade a specific tool
forge upgrade mytool

# Remove a tool and its orphaned dependencies
forge uninstall mytool --cascade
```

### Tool Info

```bash
forge info mytool
```

Shows: name, project, prerequisites, available platforms with versions, dependencies (required/recommended/optional), and recent release history.

## How Authentication Works

Forge uses **device pairing** — no API keys to generate or rotate.

1. Run `forge login` (or use the install script)
2. A pairing code appears (e.g., `ABCD-12345`)
3. Open the approval URL in your browser
4. Review the device info, select session duration, click **Approve**
5. Forge exchanges the code for a session token and saves it locally

Credentials are stored at `~/.config/forge/credentials.json` (permissions `0600`).

For unattended/CI use, set `FORGE_TOKEN` with a pre-shared API token instead.

## Dependency Resolution

When a tool has dependencies, `forge install` resolves the full graph:

```
$ forge install axxon-one-core

  Install plan:
    1. security-tools/axxon-drivers-pack  → v2.0.0  linux-amd64  2.1 MB  [required dep]
    2. security-tools/axxon-detector-pack → v1.5.0  linux-amd64  4.3 MB  [required dep]
    3. security-tools/axxon-one-core      → v20.0.0 linux-amd64  8.4 MB

  Recommended (install these too? [Y/n]):
    4. security-tools/axxon-analytics     → v1.2.0  [recommended]

  Proceed? [y/N]
```

- **Required** dependencies install automatically
- **Recommended** dependencies prompt (default yes, skip with `--skip-recommended`)
- **Optional** dependencies only with `--with-optional`

## Configuration

Config file: `~/.config/forge/config.toml`

```toml
portal_url = "https://artifacts.digitalsecurityguard.com"
default_install_path = "/opt/tools"
org_slug = "my-org"
```

**Priority order:** CLI flags > environment variables > config file > defaults.

| Env Variable | Description |
|-------------|-------------|
| `FORGE_TOKEN` | Bearer token (skips credential file) |
| `FORGE_PORTAL_URL` | Portal base URL |

## Self-Update

Forge is distributed through the portal itself:

```bash
forge upgrade forge
```

## Building from Source

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build
cd forge-cli
cargo build --release

# Binary is at target/release/forge (single static binary, ~3.6 MB)
```

## Security

- Every download is SHA-256 verified before placement on disk
- All upgrades are atomic (download to temp, verify, rename) — a failed upgrade never leaves a broken binary
- Credentials are stored with `0600` permissions in `~/.config/forge/`
- `forge verify --all` detects unauthorized modifications to installed binaries
- No post-install hooks or arbitrary code execution — Forge only places verified binaries

## License

Internal tool — Digital Security Guard / TechPro Security.

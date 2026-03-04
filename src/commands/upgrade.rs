use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{bail, Result};
use chrono::Utc;
use colored::Colorize;
use dialoguer::Confirm;

use crate::client::api::{ForgeClient, PresignLatestRequest};
use crate::client::download::download_to_file;
use crate::config::{self, Config, resolve_portal_url};
use crate::platform::detect_platform;
use crate::state::local::{InstalledTool, StateFile};

#[allow(dead_code)]
struct UpgradePlan {
    project_slug: String,
    tool_slug: String,
    old_version: Option<String>,
    new_version: Option<String>,
    new_sha256: Option<String>,
    platform_key: String,
    latest_filename: String,
    size_bytes: Option<i64>,
    path: String,
    auto_dependency: bool,
    installed_by: Option<String>,
}

pub async fn run(
    tool: Option<&str>,
    all: bool,
    yes: bool,
    portal_url_flag: Option<&str>,
) -> Result<()> {
    let config = Config::load()?;
    let portal_url = resolve_portal_url(portal_url_flag, &config);
    let token = config::require_token()?;
    let client = ForgeClient::new(&portal_url, &token)?;
    let platform = detect_platform();
    let mut state = StateFile::load()?;

    if state.installed.is_empty() {
        println!("No tools installed.");
        return Ok(());
    }

    // Determine which tools to check
    let targets: Vec<InstalledTool> = if let Some(tool_slug) = tool {
        let found = state
            .installed
            .iter()
            .find(|t| t.tool_slug == tool_slug)
            .cloned();
        match found {
            Some(t) => vec![t],
            None => bail!("Tool '{tool_slug}' is not installed."),
        }
    } else if all {
        state.installed.clone()
    } else {
        bail!("Specify a tool name or use --all to upgrade all installed tools.");
    };

    // Check each tool for updates
    let mut upgrades: Vec<UpgradePlan> = Vec::new();
    println!("  Checking for updates...");
    for installed in &targets {
        let detail = match client
            .get_tool_detail(&installed.project_slug, &installed.tool_slug)
            .await
        {
            Ok(d) => d,
            Err(e) => {
                eprintln!(
                    "  {} Could not check {}/{}: {e}",
                    "⚠".yellow(),
                    installed.project_slug,
                    installed.tool_slug
                );
                continue;
            }
        };

        let platform_info = detail
            .tool
            .platforms
            .get(&platform)
            .or_else(|| detail.tool.platforms.get("default"));

        let platform_info = match platform_info {
            Some(p) => p,
            None => continue,
        };

        // Compare SHA
        if installed.sha256.as_deref() == platform_info.sha256.as_deref()
            && platform_info.sha256.is_some()
        {
            continue; // Already up to date
        }

        let platform_key = if detail.tool.platforms.contains_key(&platform) {
            platform.clone()
        } else {
            "default".to_string()
        };

        upgrades.push(UpgradePlan {
            project_slug: installed.project_slug.clone(),
            tool_slug: installed.tool_slug.clone(),
            old_version: installed.version.clone(),
            new_version: platform_info.version.clone(),
            new_sha256: platform_info.sha256.clone(),
            platform_key,
            latest_filename: platform_info.latest_filename.clone(),
            size_bytes: platform_info.size_bytes,
            path: installed.path.clone(),
            auto_dependency: installed.auto_dependency,
            installed_by: installed.installed_by.clone(),
        });
    }

    if upgrades.is_empty() {
        println!("  {} All tools are up to date.", "✓".green());
        return Ok(());
    }

    // Display upgrade plan
    println!("\n  {}", "Upgrade plan:".bold());
    for (i, up) in upgrades.iter().enumerate() {
        let old = up.old_version.as_deref().unwrap_or("?");
        let new = up.new_version.as_deref().unwrap_or("latest");
        println!(
            "    {}. {}/{} {} → {}",
            i + 1,
            up.project_slug,
            up.tool_slug.bold(),
            old.dimmed(),
            new.green()
        );
    }

    if !yes {
        println!();
        if !Confirm::new()
            .with_prompt("  Proceed with upgrades?")
            .default(false)
            .interact()?
        {
            println!("  Aborted.");
            return Ok(());
        }
    }

    // Execute upgrades
    let mut any_failed = false;
    for up in &upgrades {
        println!(
            "\n  {} {}/{}...",
            "Upgrading".cyan(),
            up.project_slug,
            up.tool_slug
        );

        match execute_upgrade(&client, up, &mut state).await {
            Ok(()) => {
                state.save()?;
                println!(
                    "  {} {}/{}",
                    "✓".green(),
                    up.project_slug,
                    up.tool_slug
                );
            }
            Err(e) => {
                eprintln!(
                    "  {} Failed to upgrade {}/{}: {e}",
                    "✗".red(),
                    up.project_slug,
                    up.tool_slug
                );
                any_failed = true;
                // Continue with other upgrades
            }
        }
    }

    if any_failed {
        bail!("Some upgrades failed.");
    }

    println!("\n  {} All upgrades complete.", "✓".green().bold());
    Ok(())
}

async fn execute_upgrade(
    client: &ForgeClient,
    up: &UpgradePlan,
    state: &mut StateFile,
) -> Result<()> {
    let presign = client
        .presign_latest(&PresignLatestRequest {
            project: up.project_slug.clone(),
            tool: up.tool_slug.clone(),
            platform_arch: up.platform_key.clone(),
            latest_filename: up.latest_filename.clone(),
        })
        .await?;

    let final_path = PathBuf::from(&up.path);
    let tmp_path = final_path.with_extension("forge-tmp");

    let actual_sha = download_to_file(
        &presign.url,
        &tmp_path,
        presign.sha256.as_deref(),
        presign.size_bytes,
    )
    .await?;

    tokio::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755)).await?;
    tokio::fs::rename(&tmp_path, &final_path).await?;

    state.upsert(InstalledTool {
        project_slug: up.project_slug.clone(),
        tool_slug: up.tool_slug.clone(),
        version: up.new_version.clone(),
        sha256: Some(actual_sha),
        path: up.path.clone(),
        installed_at: Utc::now().to_rfc3339(),
        auto_dependency: up.auto_dependency,
        installed_by: up.installed_by.clone(),
    });

    Ok(())
}

use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use colored::Colorize;
use dialoguer::Confirm;

use crate::client::api::{
    ForgeClient, PlatformInfo, PresignLatestRequest, ToolDetailResponse,
};
use crate::client::download::download_to_file;
use crate::config::{self, Config, resolve_portal_url};
use crate::platform::detect_platform;
use crate::state::local::{InstalledTool, StateFile};

/// A resolved install step.
#[derive(Debug, Clone)]
struct InstallStep {
    project_slug: String,
    tool_slug: String,
    platform: PlatformInfo,
    platform_key: String,
    dep_type: String,        // "target", "required", "recommended", "optional"
    auto_dependency: bool,
    installed_by: Option<String>,
    prerequisites: Option<String>,
}

pub async fn run(
    tool: &str,
    project: Option<&str>,
    platform_flag: Option<&str>,
    path_flag: Option<&str>,
    portal_url_flag: Option<&str>,
    yes: bool,
    with_optional: bool,
    skip_recommended: bool,
) -> Result<()> {
    let config = Config::load()?;
    let portal_url = resolve_portal_url(portal_url_flag, &config);
    let token = config::require_token()?;
    let client = ForgeClient::new(&portal_url, &token)?;
    let install_dir = path_flag.unwrap_or(config.install_path());
    let platform = platform_flag
        .map(String::from)
        .unwrap_or_else(detect_platform);

    // Resolve project
    let project_slug = resolve_project(&client, tool, project).await?;

    // Fetch target tool detail
    let detail = client.get_tool_detail(&project_slug, tool).await?;

    // Load current state
    let mut state = StateFile::load()?;

    // Resolve dependencies (topological order, deps before target)
    let mut steps: Vec<InstallStep> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    resolve_deps(
        &client,
        &detail,
        &platform,
        &state,
        &mut steps,
        &mut visited,
        None,
        with_optional,
        skip_recommended,
    )
    .await?;

    if steps.is_empty() {
        println!("  {} is already installed and up to date.", tool.bold());
        return Ok(());
    }

    // Display plan
    display_plan(&steps)?;

    // Show prerequisites warnings
    for step in &steps {
        if let Some(prereqs) = &step.prerequisites {
            println!(
                "\n  {} Prerequisites for {}:",
                "⚠".yellow(),
                step.tool_slug.bold()
            );
            println!("    {prereqs}");
        }
    }

    // Prompt for recommended deps
    let recommended_steps: Vec<_> = steps
        .iter()
        .filter(|s| s.dep_type == "recommended")
        .collect();
    let include_recommended = if !recommended_steps.is_empty() && !yes && !skip_recommended {
        println!();
        Confirm::new()
            .with_prompt("  Install recommended dependencies too?")
            .default(true)
            .interact()?
    } else {
        !skip_recommended
    };

    // Filter steps
    let steps: Vec<_> = steps
        .into_iter()
        .filter(|s| match s.dep_type.as_str() {
            "recommended" => include_recommended,
            _ => true,
        })
        .collect();

    if steps.is_empty() {
        println!("  Nothing to install.");
        return Ok(());
    }

    // Confirm
    if !yes {
        println!();
        if !Confirm::new()
            .with_prompt("  Proceed?")
            .default(false)
            .interact()?
        {
            println!("  Aborted.");
            return Ok(());
        }
    }

    // Ensure install directory exists
    tokio::fs::create_dir_all(install_dir)
        .await
        .with_context(|| format!("Failed to create install directory: {install_dir}"))?;

    // Execute plan
    let mut any_failed = false;
    for step in &steps {
        println!(
            "\n  {} {}/{}...",
            "Installing".cyan(),
            step.project_slug,
            step.tool_slug
        );

        match execute_install_step(&client, step, install_dir, &mut state).await {
            Ok(()) => {
                state.save()?;
                println!(
                    "  {} {}/{}",
                    "✓".green(),
                    step.project_slug,
                    step.tool_slug
                );
            }
            Err(e) => {
                eprintln!(
                    "  {} Failed to install {}/{}: {e}",
                    "✗".red(),
                    step.project_slug,
                    step.tool_slug
                );
                any_failed = true;
                break;
            }
        }
    }

    if any_failed {
        bail!("Installation completed with errors.");
    }

    println!("\n  {} All tools installed successfully.", "✓".green().bold());
    Ok(())
}

async fn resolve_project(client: &ForgeClient, tool: &str, project: Option<&str>) -> Result<String> {
    match project {
        Some(p) => Ok(p.to_string()),
        None => {
            let catalog = client.get_catalog().await?;
            let mut matches: Vec<String> = Vec::new();
            for proj in &catalog.projects {
                for t in &proj.tools {
                    if t.slug == tool {
                        matches.push(proj.slug.clone());
                    }
                }
            }
            match matches.len() {
                0 => bail!("Tool '{tool}' not found in catalog."),
                1 => Ok(matches.into_iter().next().unwrap()),
                _ => bail!(
                    "Tool '{tool}' found in multiple projects: {}. Use --project to specify.",
                    matches.join(", ")
                ),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn resolve_deps(
    client: &ForgeClient,
    detail: &ToolDetailResponse,
    platform: &str,
    state: &StateFile,
    steps: &mut Vec<InstallStep>,
    visited: &mut HashSet<String>,
    installed_by: Option<&str>,
    with_optional: bool,
    skip_recommended: bool,
) -> Result<()> {
    let key = format!("{}/{}", detail.project.slug, detail.tool.slug);
    if visited.contains(&key) {
        return Ok(()); // cycle or already processed
    }
    visited.insert(key.clone());

    // Process dependencies first (topological order)
    for dep in &detail.tool.dependencies {
        let include = match dep.dependency_type.as_str() {
            "required" => true,
            "recommended" => !skip_recommended,
            "optional" => with_optional,
            _ => false,
        };
        if !include {
            continue;
        }

        let dep_key = format!("{}/{}", dep.project_slug, dep.tool_slug);
        if visited.contains(&dep_key) {
            continue;
        }

        // Fetch dep detail for its own dependencies
        let dep_detail = client
            .get_tool_detail(&dep.project_slug, &dep.tool_slug)
            .await?;

        // Recurse
        Box::pin(resolve_deps(
            client,
            &dep_detail,
            platform,
            state,
            steps,
            visited,
            Some(&key),
            with_optional,
            skip_recommended,
        ))
        .await?;
    }

    // Now add this tool itself
    let is_target = installed_by.is_none();
    let dep_type = if is_target {
        "target".to_string()
    } else {
        // Find how the parent depends on this tool
        "required".to_string()
    };

    // Check if already installed with matching SHA
    let platform_info = detail
        .tool
        .platforms
        .get(platform)
        .or_else(|| detail.tool.platforms.get("default"));

    let platform_info = match platform_info {
        Some(p) => p.clone(),
        None => {
            if is_target {
                bail!(
                    "Tool {}/{} has no build for platform '{platform}'.\nAvailable: {}",
                    detail.project.slug,
                    detail.tool.slug,
                    detail
                        .tool
                        .platforms
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            return Ok(()); // Skip dep if no platform build
        }
    };

    // Skip if already installed with same SHA
    if let Some(existing) = state.find(&detail.project.slug, &detail.tool.slug) {
        if existing.sha256.as_deref() == platform_info.sha256.as_deref()
            && platform_info.sha256.is_some()
        {
            return Ok(()); // Already up to date
        }
    }

    let platform_key = if detail.tool.platforms.contains_key(platform) {
        platform.to_string()
    } else {
        "default".to_string()
    };

    steps.push(InstallStep {
        project_slug: detail.project.slug.clone(),
        tool_slug: detail.tool.slug.clone(),
        platform: platform_info,
        platform_key,
        dep_type: if is_target { "target".to_string() } else { dep_type },
        auto_dependency: !is_target,
        installed_by: installed_by.map(String::from),
        prerequisites: detail.tool.prerequisites.clone(),
    });

    Ok(())
}

fn display_plan(steps: &[InstallStep]) -> Result<()> {
    println!("\n  {}", "Install plan:".bold());
    for (i, step) in steps.iter().enumerate() {
        let version = step
            .platform
            .version
            .as_deref()
            .unwrap_or("-");
        let size = step
            .platform
            .size_bytes
            .map(|s| format_size(s))
            .unwrap_or_else(|| "-".to_string());
        let tag = match step.dep_type.as_str() {
            "required" => "[required dep]".red().to_string(),
            "recommended" => "[recommended]".yellow().to_string(),
            "optional" => "[optional]".dimmed().to_string(),
            _ => String::new(),
        };
        println!(
            "    {}. {}/{} → {} {} {} {}",
            i + 1,
            step.project_slug,
            step.tool_slug.bold(),
            version,
            step.platform_key,
            size,
            tag
        );
    }
    Ok(())
}

async fn execute_install_step(
    client: &ForgeClient,
    step: &InstallStep,
    install_dir: &str,
    state: &mut StateFile,
) -> Result<()> {
    // Get presigned URL
    let presign = client
        .presign_latest(&PresignLatestRequest {
            project: step.project_slug.clone(),
            tool: step.tool_slug.clone(),
            platform_arch: step.platform_key.clone(),
            latest_filename: step.platform.latest_filename.clone(),
        })
        .await?;

    let final_path = PathBuf::from(install_dir).join(&presign.filename);
    let tmp_path = final_path.with_extension("forge-tmp");

    // Download + verify
    let actual_sha = download_to_file(
        &presign.url,
        &tmp_path,
        presign.sha256.as_deref(),
        presign.size_bytes,
    )
    .await?;

    // chmod 755
    tokio::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755)).await?;

    // Atomic rename
    tokio::fs::rename(&tmp_path, &final_path).await?;

    // Update state
    state.upsert(InstalledTool {
        project_slug: step.project_slug.clone(),
        tool_slug: step.tool_slug.clone(),
        version: step.platform.version.clone(),
        sha256: Some(actual_sha),
        path: final_path.to_string_lossy().to_string(),
        installed_at: Utc::now().to_rfc3339(),
        auto_dependency: step.auto_dependency,
        installed_by: step.installed_by.clone(),
    });

    Ok(())
}

fn format_size(bytes: i64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use colored::Colorize;
use dialoguer::{Confirm, MultiSelect, theme::ColorfulTheme};

use crate::client::api::{
    ForgeClient, MappingInfo, PresignLatestRequest, ToolDetailResponse,
};
use crate::client::download::download_to_file;
use crate::config::{self, Config, resolve_portal_url};
use crate::platform::detect_platform;
use crate::state::local::{InstalledTool, StateFile};

/// A resolved install step — one per file to download.
#[derive(Debug, Clone)]
struct InstallStep {
    project_slug: String,
    tool_slug: String,
    latest_filename: String,
    platform_key: String,
    version: Option<String>,
    sha256: Option<String>,
    size_bytes: Option<i64>,
    dep_type: String, // "target", "required", "recommended", "optional"
    auto_dependency: bool,
    installed_by: Option<String>,
    prerequisites: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    tool: &str,
    project: Option<&str>,
    platform_flag: Option<&str>,
    path_flag: Option<&str>,
    portal_url_flag: Option<&str>,
    yes: bool,
    with_optional: bool,
    skip_recommended: bool,
    filename_filters: Option<&str>,
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

    // Get platform mappings for the target tool
    let target_mappings = get_platform_mappings(&detail, &platform)?;

    // Select which mappings to install
    let selected_mappings = select_mappings(
        &detail,
        &target_mappings,
        filename_filters,
        yes,
    )?;

    if selected_mappings.is_empty() {
        println!("  No files selected.");
        return Ok(());
    }

    // Resolve dependencies (topological order, deps before target)
    let mut steps: Vec<InstallStep> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();

    // First resolve deps
    resolve_deps(
        &client,
        &detail,
        &platform,
        &state,
        &mut steps,
        &mut visited,
        with_optional,
        skip_recommended,
        yes,
    )
    .await?;

    // Now add the target tool's selected mappings as steps
    let platform_key = if detail.tool.platforms.contains_key(&platform) {
        platform.clone()
    } else {
        "default".to_string()
    };

    for mapping in &selected_mappings {
        // Skip if already installed with same SHA
        let already_current = state
            .installed
            .iter()
            .any(|t| {
                t.project_slug == detail.project.slug
                    && t.tool_slug == detail.tool.slug
                    && t.path.ends_with(&mapping.latest_filename)
                    && t.sha256.as_deref() == mapping.sha256.as_deref()
                    && mapping.sha256.is_some()
            });
        if already_current {
            continue;
        }

        steps.push(InstallStep {
            project_slug: detail.project.slug.clone(),
            tool_slug: detail.tool.slug.clone(),
            latest_filename: mapping.latest_filename.clone(),
            platform_key: platform_key.clone(),
            version: mapping.version.clone(),
            sha256: mapping.sha256.clone(),
            size_bytes: mapping.size_bytes,
            dep_type: "target".to_string(),
            auto_dependency: false,
            installed_by: None,
            prerequisites: detail.tool.prerequisites.clone(),
        });
    }

    if steps.is_empty() {
        println!("  {} is already installed and up to date.", tool.bold());
        return Ok(());
    }

    // Display plan
    display_plan(&steps)?;

    // Show prerequisites warnings
    let mut shown_prereqs: HashSet<String> = HashSet::new();
    for step in &steps {
        if let Some(prereqs) = &step.prerequisites {
            let key = format!("{}/{}", step.project_slug, step.tool_slug);
            if shown_prereqs.insert(key) {
                println!(
                    "\n  {} Prerequisites for {}:",
                    "⚠".yellow(),
                    step.tool_slug.bold()
                );
                println!("    {prereqs}");
            }
        }
    }

    // Prompt for recommended deps
    let has_recommended = steps.iter().any(|s| s.dep_type == "recommended");
    let include_recommended = if has_recommended && !yes && !skip_recommended {
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
    if tokio::fs::create_dir_all(install_dir).await.is_err() {
        // Try with sudo (common for /opt/tools)
        let status = std::process::Command::new("sudo")
            .args(["mkdir", "-p", install_dir])
            .status()
            .with_context(|| format!("Failed to create install directory: {install_dir}"))?;
        if !status.success() {
            bail!("Failed to create install directory: {install_dir}");
        }
        // Make it writable by current user
        let user = std::env::var("USER").unwrap_or_else(|_| "root".to_string());
        let _ = std::process::Command::new("sudo")
            .args(["chown", &user, install_dir])
            .status();
    }

    // Execute plan
    let mut any_failed = false;
    for step in &steps {
        println!(
            "\n  {} {}/{}  {}...",
            "Installing".cyan(),
            step.project_slug,
            step.tool_slug,
            step.latest_filename.dimmed()
        );

        match execute_install_step(&client, step, install_dir, &mut state).await {
            Ok(()) => {
                state.save()?;
                println!(
                    "  {} {}/{}  {}",
                    "✓".green(),
                    step.project_slug,
                    step.tool_slug,
                    step.latest_filename
                );
            }
            Err(e) => {
                eprintln!(
                    "  {} Failed to install {}/{} ({}): {e}",
                    "✗".red(),
                    step.project_slug,
                    step.tool_slug,
                    step.latest_filename
                );
                any_failed = true;
                break;
            }
        }
    }

    if any_failed {
        bail!("Installation completed with errors.");
    }

    println!("\n  {} All files installed successfully.", "✓".green().bold());
    Ok(())
}

/// Get all mappings for the given platform from the tool detail.
fn get_platform_mappings(
    detail: &ToolDetailResponse,
    platform: &str,
) -> Result<Vec<MappingInfo>> {
    // Use the mappings array if available (supports multiple files per platform)
    if !detail.tool.mappings.is_empty() {
        let matches: Vec<MappingInfo> = detail
            .tool
            .mappings
            .iter()
            .filter(|m| m.platform_arch == platform || m.platform_arch == "default")
            .cloned()
            .collect();
        if !matches.is_empty() {
            return Ok(matches);
        }
    }

    // Fallback to platforms dict (single mapping per platform, backward compat)
    let platform_info = detail
        .tool
        .platforms
        .get(platform)
        .or_else(|| detail.tool.platforms.get("default"));

    match platform_info {
        Some(p) => Ok(vec![MappingInfo {
            platform_arch: platform.to_string(),
            latest_filename: p.latest_filename.clone(),
            latest_url: p.latest_url.clone(),
            version: p.version.clone(),
            sha256: p.sha256.clone(),
            size_bytes: p.size_bytes,
        }]),
        None => bail!(
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
        ),
    }
}

/// Let the user select which mappings to install.
fn select_mappings(
    detail: &ToolDetailResponse,
    mappings: &[MappingInfo],
    filename_filter: Option<&str>,
    yes: bool,
) -> Result<Vec<MappingInfo>> {
    // If only one mapping, use it directly
    if mappings.len() == 1 {
        return Ok(mappings.to_vec());
    }

    // If --filename specified, filter to matching
    if let Some(filter) = filename_filter {
        let matches: Vec<MappingInfo> = mappings
            .iter()
            .filter(|m| m.latest_filename == filter)
            .cloned()
            .collect();
        if matches.is_empty() {
            bail!(
                "No mapping with filename '{}' found for {}/{}.\nAvailable: {}",
                filter,
                detail.project.slug,
                detail.tool.slug,
                mappings
                    .iter()
                    .map(|m| m.latest_filename.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        return Ok(matches);
    }

    // If --yes, install all
    if yes {
        return Ok(mappings.to_vec());
    }

    // Interactive: show checklist
    println!(
        "\n  {} has {} files available:",
        format!("{}/{}", detail.project.slug, detail.tool.slug).bold(),
        mappings.len()
    );

    let items: Vec<String> = mappings
        .iter()
        .map(|m| {
            let version = m.version.as_deref().unwrap_or("-");
            let size = m
                .size_bytes
                .map(|s| format_size(s))
                .unwrap_or_else(|| "-".to_string());
            format!("{:<40} {} {}", m.latest_filename, version, size)
        })
        .collect();

    // Default: all selected
    let defaults: Vec<bool> = vec![true; items.len()];

    let selections = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("  Select files to install (space to toggle, enter to confirm)")
        .items(&items)
        .defaults(&defaults)
        .interact()?;

    let selected: Vec<MappingInfo> = selections
        .into_iter()
        .map(|i| mappings[i].clone())
        .collect();

    Ok(selected)
}

async fn resolve_project(
    client: &ForgeClient,
    tool: &str,
    project: Option<&str>,
) -> Result<String> {
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

/// Resolve dependency steps (for deps, use first mapping per platform).
#[allow(clippy::too_many_arguments)]
async fn resolve_deps(
    client: &ForgeClient,
    detail: &ToolDetailResponse,
    platform: &str,
    state: &StateFile,
    steps: &mut Vec<InstallStep>,
    visited: &mut HashSet<String>,
    with_optional: bool,
    skip_recommended: bool,
    yes: bool,
) -> Result<()> {
    let key = format!("{}/{}", detail.project.slug, detail.tool.slug);
    if visited.contains(&key) {
        return Ok(());
    }
    visited.insert(key.clone());

    // Process dependencies (topological order — deps before this tool)
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

        let dep_detail = client
            .get_tool_detail(&dep.project_slug, &dep.tool_slug)
            .await?;

        // Recurse for the dep's own deps
        Box::pin(resolve_deps(
            client,
            &dep_detail,
            platform,
            state,
            steps,
            visited,
            with_optional,
            skip_recommended,
            yes,
        ))
        .await?;

        // Get platform mappings for this dep
        let dep_mappings = match get_platform_mappings(&dep_detail, platform) {
            Ok(m) => m,
            Err(_) => continue, // Skip dep if no platform build
        };

        // Let user select which files to install (same UX as target tool)
        let selected = select_mappings(&dep_detail, &dep_mappings, None, yes)?;
        if selected.is_empty() {
            continue;
        }

        let platform_key = if dep_detail.tool.platforms.contains_key(platform) {
            platform.to_string()
        } else {
            "default".to_string()
        };

        for mapping in &selected {
            // Skip if already installed with same SHA
            let already_current = state
                .installed
                .iter()
                .any(|t| {
                    t.project_slug == dep_detail.project.slug
                        && t.tool_slug == dep_detail.tool.slug
                        && t.path.ends_with(&mapping.latest_filename)
                        && t.sha256.as_deref() == mapping.sha256.as_deref()
                        && mapping.sha256.is_some()
                });
            if already_current {
                continue;
            }

            steps.push(InstallStep {
                project_slug: dep_detail.project.slug.clone(),
                tool_slug: dep_detail.tool.slug.clone(),
                latest_filename: mapping.latest_filename.clone(),
                platform_key: platform_key.clone(),
                version: mapping.version.clone(),
                sha256: mapping.sha256.clone(),
                size_bytes: mapping.size_bytes,
                dep_type: dep.dependency_type.clone(),
                auto_dependency: true,
                installed_by: Some(key.clone()),
                prerequisites: dep_detail.tool.prerequisites.clone(),
            });
        }
    }

    Ok(())
}

fn display_plan(steps: &[InstallStep]) -> Result<()> {
    println!("\n  {}", "Install plan:".bold());
    for (i, step) in steps.iter().enumerate() {
        let version = step.version.as_deref().unwrap_or("-");
        let size = step
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
            "    {}. {}/{}  {} → {} {} {}",
            i + 1,
            step.project_slug,
            step.latest_filename.bold(),
            step.tool_slug.dimmed(),
            version,
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
    let presign = client
        .presign_latest(&PresignLatestRequest {
            project: step.project_slug.clone(),
            tool: step.tool_slug.clone(),
            platform_arch: step.platform_key.clone(),
            latest_filename: step.latest_filename.clone(),
        })
        .await?;

    let final_path = PathBuf::from(install_dir).join(&presign.filename);
    let tmp_path = final_path.with_extension("forge-tmp");

    let actual_sha = download_to_file(
        &presign.url,
        &tmp_path,
        presign.sha256.as_deref(),
        presign.size_bytes,
    )
    .await?;

    // Atomic rename
    tokio::fs::rename(&tmp_path, &final_path).await?;

    // Post-download: install based on file extension
    let filename = presign.filename.to_lowercase();
    if filename.ends_with(".deb") {
        // Install Debian package
        println!("    {} Running dpkg -i {}...", "→".cyan(), presign.filename);
        let status = std::process::Command::new("sudo")
            .args(["dpkg", "-i", &final_path.to_string_lossy()])
            .status()
            .context("Failed to run dpkg")?;
        if !status.success() {
            // Try to fix broken dependencies
            println!("    {} Fixing dependencies with apt-get -f install...", "→".cyan());
            let fix_status = std::process::Command::new("sudo")
                .args(["apt-get", "-f", "install", "-y"])
                .status()
                .context("Failed to run apt-get -f install")?;
            if !fix_status.success() {
                bail!("dpkg -i failed for {} and apt-get -f install could not fix it", presign.filename);
            }
        }
    } else if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
        // Extract tarball to install dir
        println!("    {} Extracting {}...", "→".cyan(), presign.filename);
        let status = std::process::Command::new("tar")
            .args(["xzf", &final_path.to_string_lossy(), "-C", install_dir])
            .status()
            .context("Failed to extract tarball")?;
        if !status.success() {
            bail!("Failed to extract {}", presign.filename);
        }
    } else {
        // Raw binary or unknown — just make executable
        tokio::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o755)).await?;
    }

    // Update state
    state.upsert(InstalledTool {
        project_slug: step.project_slug.clone(),
        tool_slug: step.tool_slug.clone(),
        version: step.version.clone(),
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

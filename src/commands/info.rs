use anyhow::{bail, Result};
use colored::Colorize;

use crate::client::api::ForgeClient;
use crate::config::{self, Config, resolve_portal_url};

pub async fn run(
    tool: &str,
    project: Option<&str>,
    portal_url_flag: Option<&str>,
) -> Result<()> {
    let config = Config::load()?;
    let portal_url = resolve_portal_url(portal_url_flag, &config);
    let token = config::require_token()?;
    let client = ForgeClient::new(&portal_url, &token)?;

    // Resolve project slug
    let project_slug = match project {
        Some(p) => p.to_string(),
        None => {
            // Search catalog for unique match
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
                1 => matches.into_iter().next().unwrap(),
                _ => bail!(
                    "Tool '{tool}' found in multiple projects: {}. Use --project to specify.",
                    matches.join(", ")
                ),
            }
        }
    };

    let detail = client.get_tool_detail(&project_slug, tool).await?;
    let t = &detail.tool;

    println!();
    println!("  {} {}/{}", "Tool:".bold(), detail.project.slug, t.slug);
    println!("  {} {}", "Name:".bold(), t.name);
    println!(
        "  {} {}",
        "Project:".bold(),
        detail.project.name
    );

    if let Some(prereqs) = &t.prerequisites {
        println!("  {} {}", "Prerequisites:".bold(), prereqs);
    }

    println!();
    println!("  {}", "Platforms:".bold());
    for (platform, info) in &t.platforms {
        let version = info.version.as_deref().unwrap_or("-");
        let size = info
            .size_bytes
            .map(|s| format_size(s))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "    {:<16} version: {:<10} size: {}",
            platform, version, size
        );
    }

    if !t.dependencies.is_empty() {
        println!();
        println!("  {}", "Dependencies:".bold());
        for dep in &t.dependencies {
            let dtype = match dep.dependency_type.as_str() {
                "required" => "required".red().to_string(),
                "recommended" => "recommended".yellow().to_string(),
                "optional" => "optional".dimmed().to_string(),
                other => other.to_string(),
            };
            println!(
                "    {}/{} ({})",
                dep.project_slug, dep.tool_slug, dtype
            );
        }
    }

    if !t.releases.is_empty() {
        println!();
        println!("  {} (latest {})", "Releases:".bold(), t.releases.len());
        for release in t.releases.iter().take(5) {
            let version = release.version.as_deref().unwrap_or("-");
            let date = release
                .created_at
                .as_deref()
                .and_then(|d| d.split('T').next())
                .unwrap_or("-");
            let notes = release.notes.as_deref().unwrap_or("");
            if notes.is_empty() {
                println!("    {} ({})", version, date);
            } else {
                println!("    {} ({}) — {}", version, date, notes);
            }
        }
    }

    println!();
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

use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::Confirm;

use crate::state::local::StateFile;

pub async fn run(
    tool: &str,
    project: Option<&str>,
    force: bool,
    cascade: bool,
) -> Result<()> {
    let mut state = StateFile::load()?;

    // Find the tool
    let installed = if let Some(proj) = project {
        state.find(proj, tool).cloned()
    } else {
        let matches: Vec<_> = state
            .installed
            .iter()
            .filter(|t| t.tool_slug == tool)
            .collect();
        match matches.len() {
            0 => None,
            1 => Some(matches[0].clone()),
            _ => bail!(
                "Tool '{tool}' found in multiple projects: {}. Use --project to specify.",
                matches
                    .iter()
                    .map(|t| t.project_slug.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    };

    let installed = match installed {
        Some(t) => t,
        None => bail!("Tool '{tool}' is not installed."),
    };

    // Check reverse deps
    let dependents = state.dependents_of(&installed.project_slug, &installed.tool_slug);
    if !dependents.is_empty() && !force && !cascade {
        println!(
            "  {} Cannot uninstall {}/{} — the following tools depend on it:",
            "✗".red(),
            installed.project_slug,
            installed.tool_slug
        );
        for dep in &dependents {
            println!("    - {}/{}", dep.project_slug, dep.tool_slug);
        }
        println!("\n  Use --force to remove anyway, or --cascade to remove dependents too.");
        bail!("Cannot uninstall: has dependents.");
    }

    // Collect removal targets
    let mut to_remove = vec![(
        installed.project_slug.clone(),
        installed.tool_slug.clone(),
        installed.path.clone(),
    )];

    if cascade {
        // Also remove auto-deps installed by this tool
        let deps =
            state.dependencies_installed_by(&installed.project_slug, &installed.tool_slug);
        for dep in deps {
            to_remove.push((
                dep.project_slug.clone(),
                dep.tool_slug.clone(),
                dep.path.clone(),
            ));
        }
    }

    // Display plan
    println!("\n  {}", "Uninstall plan:".bold());
    for (proj, tool_slug, path) in &to_remove {
        println!("    - {proj}/{tool_slug} ({})", path);
    }

    println!();
    if !Confirm::new()
        .with_prompt("  Proceed with removal?")
        .default(false)
        .interact()?
    {
        println!("  Aborted.");
        return Ok(());
    }

    // Remove files and state
    for (proj, tool_slug, path) in &to_remove {
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!(
                    "  {} File already missing: {path}",
                    "⚠".yellow()
                );
            }
            Err(e) => {
                eprintln!(
                    "  {} Failed to remove {path}: {e}",
                    "⚠".yellow()
                );
            }
        }
        state.remove(proj, tool_slug);
    }

    state.save()?;
    println!(
        "  {} Removed {} tool(s).",
        "✓".green().bold(),
        to_remove.len()
    );

    Ok(())
}

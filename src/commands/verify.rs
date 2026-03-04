use anyhow::{bail, Result};
use colored::Colorize;
use sha2::{Digest, Sha256};

use crate::state::local::StateFile;

pub async fn run(tool: Option<&str>, all: bool) -> Result<()> {
    let state = StateFile::load()?;

    if state.installed.is_empty() {
        println!("No tools installed.");
        return Ok(());
    }

    let targets: Vec<_> = if let Some(tool_slug) = tool {
        let found: Vec<_> = state
            .installed
            .iter()
            .filter(|t| t.tool_slug == tool_slug)
            .collect();
        if found.is_empty() {
            bail!("Tool '{tool_slug}' is not installed.");
        }
        found
    } else if all {
        state.installed.iter().collect()
    } else {
        bail!("Specify a tool name or use --all to verify all installed tools.");
    };

    let mut failures = 0;

    for installed in &targets {
        let label = format!("{}/{}", installed.project_slug, installed.tool_slug);

        // Check if file exists
        let path = std::path::Path::new(&installed.path);
        if !path.exists() {
            println!("  {} {} — {}", "MISSING".red().bold(), label, installed.path);
            failures += 1;
            continue;
        }

        // No stored hash to compare
        let expected = match &installed.sha256 {
            Some(h) => h,
            None => {
                println!(
                    "  {} {} — no hash recorded",
                    "SKIP".yellow().bold(),
                    label
                );
                continue;
            }
        };

        // Compute SHA-256
        let data = tokio::fs::read(&installed.path).await?;
        let actual = format!("{:x}", Sha256::digest(&data));

        if actual == *expected {
            println!("  {} {}", "OK".green().bold(), label);
        } else {
            println!(
                "  {} {} — hash mismatch",
                "MODIFIED".red().bold(),
                label
            );
            failures += 1;
        }
    }

    if failures > 0 {
        bail!("{failures} verification failure(s).");
    }

    Ok(())
}

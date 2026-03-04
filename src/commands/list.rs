use anyhow::Result;
use colored::Colorize;

use crate::state::local::StateFile;

pub async fn run() -> Result<()> {
    let state = StateFile::load()?;

    if state.installed.is_empty() {
        println!("No tools installed.");
        return Ok(());
    }

    println!(
        "  {:<30} {:<12} {:<30} {}",
        "TOOL".bold(),
        "VERSION".bold(),
        "PATH".bold(),
        "INSTALLED".bold()
    );
    println!("  {}", "─".repeat(80));

    for tool in &state.installed {
        let name = if tool.auto_dependency {
            format!("{}/{} {}", tool.project_slug, tool.tool_slug, "[dep]".dimmed())
        } else {
            format!("{}/{}", tool.project_slug, tool.tool_slug)
        };
        let version = tool.version.as_deref().unwrap_or("-");
        let date = tool
            .installed_at
            .split('T')
            .next()
            .unwrap_or(&tool.installed_at);
        println!("  {:<30} {:<12} {:<30} {}", name, version, tool.path, date);
    }

    Ok(())
}

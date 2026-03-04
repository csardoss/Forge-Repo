use anyhow::Result;
use colored::Colorize;

use crate::client::api::ForgeClient;
use crate::config::{self, Config, resolve_portal_url};

pub async fn run(portal_url_flag: Option<&str>, json: bool) -> Result<()> {
    let config = Config::load()?;
    let portal_url = resolve_portal_url(portal_url_flag, &config);
    let token = config::require_token()?;
    let client = ForgeClient::new(&portal_url, &token)?;
    let catalog = client.get_catalog().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&catalog.projects)?);
        return Ok(());
    }

    if catalog.projects.is_empty() {
        println!("No registry-enabled tools found.");
        return Ok(());
    }

    println!(
        "  {:<20} {:<20} {:<12} {}",
        "PROJECT".bold(),
        "TOOL".bold(),
        "VERSION".bold(),
        "PLATFORMS".bold()
    );
    println!("  {}", "─".repeat(72));

    for project in &catalog.projects {
        for tool in &project.tools {
            let platforms: Vec<String> = tool.platforms.keys().cloned().collect();
            let version = tool
                .platforms
                .values()
                .next()
                .and_then(|p| p.version.clone())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "  {:<20} {:<20} {:<12} {}",
                project.slug,
                tool.slug,
                version,
                platforms.join(", ")
            );
        }
    }

    Ok(())
}

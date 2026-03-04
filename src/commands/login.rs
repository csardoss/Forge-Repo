use anyhow::{Context, Result};
use colored::Colorize;
use dialoguer::{Input, theme::ColorfulTheme};
use indicatif::{ProgressBar, ProgressStyle};

use crate::client::api::{
    ForgeClient, PairingExchangeRequest, PairingMetadata, PairingStartRequest,
};
use crate::config::{Config, Credentials, resolve_portal_url};
use crate::platform::detect_platform;

pub async fn run(portal_url_flag: Option<&str>) -> Result<()> {
    let config = Config::load()?;
    let default_url = resolve_portal_url(portal_url_flag, &config);

    let portal_url: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Portal URL")
        .default(default_url)
        .interact_text()?;

    let default_org = config.org_slug.clone().unwrap_or_default();
    let org_slug: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Organization slug")
        .default(default_org)
        .interact_text()?;

    let client = ForgeClient::anonymous(&portal_url)?;

    let platform = detect_platform();
    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let start_resp = client
        .pairing_start(&PairingStartRequest {
            org_slug: org_slug.clone(),
            app_id: "forge-cli".to_string(),
            instance_id: host.clone(),
            requested_scopes: vec![
                "registry:read".to_string(),
                "download".to_string(),
                "manifest:read".to_string(),
                "latest:read".to_string(),
            ],
            metadata: PairingMetadata {
                hostname: host,
                platform: platform.clone(),
                arch: std::env::consts::ARCH.to_string(),
            },
        })
        .await
        .context("Failed to start pairing")?;

    println!();
    println!(
        "  Pairing code:  {}",
        start_resp.pairing_code.bold().cyan()
    );
    println!(
        "  Approve at:    {}",
        start_resp.pairing_url.underline()
    );
    println!();

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("  {spinner} Waiting for approval...")
            .unwrap(),
    );
    spinner.enable_steady_tick(std::time::Duration::from_millis(120));

    // Poll every 2 seconds
    let exchange_token;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let status = client
            .pairing_status(&start_resp.pairing_code)
            .await
            .context("Failed to poll pairing status")?;

        match status.status.as_str() {
            "approved" => {
                exchange_token = status
                    .exchange_token
                    .context("Approved but no exchange token received")?;
                break;
            }
            "denied" => {
                spinner.finish_and_clear();
                anyhow::bail!("Pairing was denied.");
            }
            "expired" => {
                spinner.finish_and_clear();
                anyhow::bail!("Pairing code expired. Run `forge login` again.");
            }
            "exchanged" => {
                spinner.finish_and_clear();
                anyhow::bail!("Pairing code was already exchanged.");
            }
            "pending" => continue,
            other => {
                spinner.finish_and_clear();
                anyhow::bail!("Unexpected pairing status: {other}");
            }
        }
    }
    spinner.finish_and_clear();

    // Exchange immediately (60s window)
    let exchange_resp = client
        .pairing_exchange(&PairingExchangeRequest {
            pairing_code: start_resp.pairing_code,
            exchange_token,
        })
        .await
        .context("Failed to exchange pairing code for token")?;

    let creds = Credentials {
        access_token: exchange_resp.access_token,
        expires_at: Some(exchange_resp.expires_at.clone()),
        scopes: exchange_resp.scopes,
        portal_url: portal_url.clone(),
        org_slug: org_slug.clone(),
    };
    creds.save().context("Failed to save credentials")?;

    println!("{}", "  Login successful!".green().bold());
    println!("  Token expires: {}", exchange_resp.expires_at);
    println!("  Credentials saved to ~/.config/forge/credentials.json");

    Ok(())
}

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const DEFAULT_PORTAL_URL: &str = "https://artifacts.digitalsecurityguard.com";
const DEFAULT_INSTALL_PATH: &str = "/opt/tools";

/// User configuration from ~/.config/forge/config.toml
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Config {
    pub portal_url: Option<String>,
    pub default_install_path: Option<String>,
    pub org_slug: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::config_dir()?.join("config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        toml::from_str(&contents).with_context(|| "Failed to parse config.toml")
    }

    pub fn install_path(&self) -> &str {
        self.default_install_path
            .as_deref()
            .unwrap_or(DEFAULT_INSTALL_PATH)
    }

    pub fn config_dir() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("forge");
        if !dir.exists() {
            fs::create_dir_all(&dir)
                .with_context(|| format!("Failed to create {}", dir.display()))?;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
        Ok(dir)
    }
}

/// Saved credentials from ~/.config/forge/credentials.json
#[derive(Debug, Deserialize, Serialize)]
pub struct Credentials {
    pub access_token: String,
    pub expires_at: Option<String>,
    pub scopes: Vec<String>,
    pub portal_url: String,
    pub org_slug: String,
}

impl Credentials {
    pub fn load() -> Result<Option<Self>> {
        let path = Config::config_dir()?.join("credentials.json");
        if !path.exists() {
            return Ok(None);
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let creds: Self =
            serde_json::from_str(&contents).with_context(|| "Failed to parse credentials.json")?;
        Ok(Some(creds))
    }

    pub fn save(&self) -> Result<()> {
        let dir = Config::config_dir()?;
        let path = dir.join("credentials.json");
        let tmp = dir.join("credentials.json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&tmp, &json)?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn is_expired(&self) -> bool {
        match &self.expires_at {
            None => false,
            Some(s) => match s.parse::<DateTime<Utc>>() {
                Ok(dt) => dt < Utc::now(),
                Err(_) => false,
            },
        }
    }
}

/// Resolve portal URL: --portal-url flag > FORGE_PORTAL_URL env > config > default.
pub fn resolve_portal_url(flag: Option<&str>, config: &Config) -> String {
    if let Some(url) = flag {
        return url.to_string();
    }
    if let Ok(url) = std::env::var("FORGE_PORTAL_URL") {
        return url;
    }
    config
        .portal_url
        .clone()
        .unwrap_or_else(|| DEFAULT_PORTAL_URL.to_string())
}

/// Resolve token: FORGE_TOKEN env > credentials file.
pub fn resolve_token() -> Result<Option<String>> {
    if let Ok(token) = std::env::var("FORGE_TOKEN") {
        return Ok(Some(token));
    }
    match Credentials::load()? {
        Some(creds) => {
            if creds.is_expired() {
                anyhow::bail!(crate::error::ForgeError::TokenExpired);
            }
            Ok(Some(creds.access_token))
        }
        None => Ok(None),
    }
}

/// Resolve token, returning an error if not available.
pub fn require_token() -> Result<String> {
    resolve_token()?.ok_or_else(|| crate::error::ForgeError::NotAuthenticated.into())
}

#[allow(dead_code)]
/// Resolve the org slug from credentials.
pub fn resolve_org_slug() -> Result<Option<String>> {
    if let Ok(creds) = Credentials::load() {
        if let Some(c) = creds {
            return Ok(Some(c.org_slug));
        }
    }
    Ok(None)
}

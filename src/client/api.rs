use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};

use crate::error::ForgeError;

/// Typed API client for the artifact portal.
pub struct ForgeClient {
    http: reqwest::Client,
    base_url: String,
}

// ── Response types matching server JSON ──────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct CatalogResponse {
    pub projects: Vec<CatalogProject>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CatalogProject {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub tools: Vec<CatalogTool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CatalogTool {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub prerequisites: Option<String>,
    pub platforms: std::collections::HashMap<String, PlatformInfo>,
    #[serde(default)]
    pub mappings: Vec<MappingInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlatformInfo {
    pub latest_filename: String,
    pub latest_url: Option<String>,
    pub version: Option<String>,
    pub sha256: Option<String>,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ToolDetailResponse {
    pub project: ToolDetailProject,
    pub tool: ToolDetailTool,
}

#[derive(Debug, Deserialize)]
pub struct ToolDetailProject {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Deserialize)]
pub struct ToolDetailTool {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub prerequisites: Option<String>,
    pub platforms: std::collections::HashMap<String, PlatformInfo>,
    #[serde(default)]
    pub mappings: Vec<MappingInfo>,
    pub dependencies: Vec<ToolDependency>,
    pub releases: Vec<ReleaseInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MappingInfo {
    pub platform_arch: String,
    pub latest_filename: String,
    pub latest_url: Option<String>,
    pub version: Option<String>,
    pub sha256: Option<String>,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolDependency {
    pub tool_id: i64,
    pub tool_name: String,
    pub tool_slug: String,
    pub project_name: String,
    pub project_slug: String,
    pub dependency_type: String,
    pub sort_order: i32,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseInfo {
    pub id: i64,
    pub version: Option<String>,
    pub created_at: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PresignResponse {
    pub url: String,
    pub expires_at: String,
    pub sha256: Option<String>,
    pub size_bytes: Option<i64>,
    pub filename: String,
}

// ── Pairing types ────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PairingStartRequest {
    pub org_slug: String,
    pub app_id: String,
    pub instance_id: String,
    pub requested_scopes: Vec<String>,
    pub metadata: PairingMetadata,
}

#[derive(Debug, Serialize)]
pub struct PairingMetadata {
    pub hostname: String,
    pub platform: String,
    pub arch: String,
}

#[derive(Debug, Deserialize)]
pub struct PairingStartResponse {
    pub pairing_code: String,
    pub pairing_url: String,
    pub expires_in: i64,
}

#[derive(Debug, Deserialize)]
pub struct PairingStatusResponse {
    pub status: String,
    pub exchange_token: Option<String>,
    pub exchange_expires_in: Option<i64>,
    pub effective_ttl_seconds: Option<i64>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PairingExchangeRequest {
    pub pairing_code: String,
    pub exchange_token: String,
}

#[derive(Debug, Deserialize)]
pub struct PairingExchangeResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_at: String,
    pub scopes: Vec<String>,
}

// ── Presign request ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PresignLatestRequest {
    pub project: String,
    pub tool: String,
    pub platform_arch: String,
    pub latest_filename: String,
}

// ── Client implementation ────────────────────────────────────────────

impl ForgeClient {
    /// Create an authenticated client.
    pub fn new(portal_url: &str, token: &str) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("forge/0.2.0"));
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .context("Invalid token characters")?,
        );
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;
        let base_url = portal_url.trim_end_matches('/').to_string();
        Ok(Self { http, base_url })
    }

    /// Create an anonymous client (for pairing endpoints).
    pub fn anonymous(portal_url: &str) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("forge/0.2.0"));
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;
        let base_url = portal_url.trim_end_matches('/').to_string();
        Ok(Self { http, base_url })
    }

    /// Return the base URL (for display).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Handle API response errors.
    async fn handle_response(&self, resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status().as_u16();
        if status >= 400 {
            let body = resp.text().await.unwrap_or_default();
            let detail = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("detail").and_then(|d| d.as_str()).map(String::from))
                .unwrap_or(body);
            match status {
                401 => anyhow::bail!(ForgeError::NotAuthenticated),
                403 => anyhow::bail!(ForgeError::Forbidden(detail)),
                404 => anyhow::bail!(ForgeError::NotFound(detail)),
                _ => anyhow::bail!(ForgeError::ApiError(status, detail)),
            }
        }
        Ok(resp)
    }

    // ── Pairing ──────────────────────────────────────────────────────

    pub async fn pairing_start(&self, req: &PairingStartRequest) -> Result<PairingStartResponse> {
        let resp = self
            .http
            .post(format!("{}/api/v2/pairing/start", self.base_url))
            .json(req)
            .send()
            .await
            .context("Failed to connect to portal")?;
        let resp = self.handle_response(resp).await?;
        resp.json().await.context("Failed to parse pairing response")
    }

    pub async fn pairing_status(&self, code: &str) -> Result<PairingStatusResponse> {
        let resp = self
            .http
            .get(format!("{}/api/v2/pairing/status/{code}", self.base_url))
            .send()
            .await
            .context("Failed to poll pairing status")?;
        let resp = self.handle_response(resp).await?;
        resp.json().await.context("Failed to parse status response")
    }

    pub async fn pairing_exchange(
        &self,
        req: &PairingExchangeRequest,
    ) -> Result<PairingExchangeResponse> {
        let resp = self
            .http
            .post(format!("{}/api/v2/pairing/exchange", self.base_url))
            .json(req)
            .send()
            .await
            .context("Failed to exchange pairing code")?;
        let resp = self.handle_response(resp).await?;
        resp.json()
            .await
            .context("Failed to parse exchange response")
    }

    // ── Registry ─────────────────────────────────────────────────────

    pub async fn get_catalog(&self) -> Result<CatalogResponse> {
        let resp = self
            .http
            .get(format!("{}/api/v2/registry/catalog", self.base_url))
            .send()
            .await
            .context("Failed to fetch catalog")?;
        let resp = self.handle_response(resp).await?;
        resp.json().await.context("Failed to parse catalog")
    }

    pub async fn get_tool_detail(
        &self,
        project_slug: &str,
        tool_slug: &str,
    ) -> Result<ToolDetailResponse> {
        let resp = self
            .http
            .get(format!(
                "{}/api/v2/registry/tool/{project_slug}/{tool_slug}",
                self.base_url
            ))
            .send()
            .await
            .context("Failed to fetch tool detail")?;
        let resp = self.handle_response(resp).await?;
        resp.json().await.context("Failed to parse tool detail")
    }

    pub async fn presign_latest(&self, req: &PresignLatestRequest) -> Result<PresignResponse> {
        let resp = self
            .http
            .post(format!("{}/api/v2/presign-latest", self.base_url))
            .json(req)
            .send()
            .await
            .context("Failed to presign download")?;
        let resp = self.handle_response(resp).await?;
        resp.json().await.context("Failed to parse presign response")
    }
}

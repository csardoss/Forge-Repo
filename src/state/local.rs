use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct StateFile {
    pub version: u32,
    pub installed: Vec<InstalledTool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstalledTool {
    pub project_slug: String,
    pub tool_slug: String,
    pub version: Option<String>,
    pub sha256: Option<String>,
    pub path: String,
    pub installed_at: String,
    pub auto_dependency: bool,
    pub installed_by: Option<String>,
}

impl StateFile {
    fn path() -> Result<PathBuf> {
        Ok(Config::config_dir()?.join("state.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self {
                version: 1,
                installed: Vec::new(),
            });
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        serde_json::from_str(&contents).with_context(|| "Failed to parse state.json")
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&tmp, &json)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn find(&self, project_slug: &str, tool_slug: &str) -> Option<&InstalledTool> {
        self.installed
            .iter()
            .find(|t| t.project_slug == project_slug && t.tool_slug == tool_slug)
    }

    pub fn upsert(&mut self, tool: InstalledTool) {
        if let Some(existing) = self
            .installed
            .iter_mut()
            .find(|t| t.project_slug == tool.project_slug && t.tool_slug == tool.tool_slug)
        {
            *existing = tool;
        } else {
            self.installed.push(tool);
        }
    }

    pub fn remove(&mut self, project_slug: &str, tool_slug: &str) -> Option<InstalledTool> {
        let idx = self
            .installed
            .iter()
            .position(|t| t.project_slug == project_slug && t.tool_slug == tool_slug)?;
        Some(self.installed.remove(idx))
    }

    /// Find tools that were auto-installed by a given tool.
    pub fn dependencies_installed_by(&self, project_slug: &str, tool_slug: &str) -> Vec<&InstalledTool> {
        let key = format!("{project_slug}/{tool_slug}");
        self.installed
            .iter()
            .filter(|t| t.installed_by.as_deref() == Some(&key))
            .collect()
    }

    /// Find tools that depend on a given tool (installed_by points to it).
    pub fn dependents_of(&self, project_slug: &str, tool_slug: &str) -> Vec<&InstalledTool> {
        // A tool T is a dependent of X if T.installed_by == "X_project/X_tool"
        // OR if some other installed tool was installed_by T and T depends on X.
        // For simplicity, we only check direct installed_by references.
        let key = format!("{project_slug}/{tool_slug}");
        self.installed
            .iter()
            .filter(|t| t.installed_by.as_deref() == Some(&key))
            .collect()
    }
}

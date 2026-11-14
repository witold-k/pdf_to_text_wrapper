// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Witold Kaminski

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::{env, fs, path::{Path, PathBuf}};

use crate::process_pdf_to_text::PdfBackend;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrobidConfig {
    pub source: String,
    pub url: String,
    pub server_binary: PathBuf,
    pub server_workdir: PathBuf,
    pub server_args: Vec<String>,
    pub threads: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MineruConfig {
    pub source: String,
    pub binary: PathBuf,
    pub server_binary: PathBuf,
    pub server_workdir: PathBuf,
    pub server_args: Vec<String>,
    pub threads: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Filename suffixes accepted by source scanning (as in aiagents).
    pub scanendfilter: Vec<String>,
    pub backend: PdfBackend,
    pub grobid: GrobidConfig,
    pub mineru: MineruConfig,
}

fn default_scanendfilter() -> Vec<String> {
    vec![".pdf".into(), ".docx".into()]
}

impl Default for Config {
    fn default() -> Self {
        let services_dir = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("~"))
            .join("opt/services");

        Self {
            scanendfilter: default_scanendfilter(),
            backend: PdfBackend::Grobid,
            grobid: GrobidConfig {
                source: "https://github.com/grobidOrg/grobid".into(),
                url: "http://localhost:8070/api/processFulltextDocument".into(),
                server_binary: services_dir.join("grobid/gradlew"),
                server_workdir: services_dir.join("grobid"),
                server_args: vec![":grobid-service:run".into()],
                threads: 1,
            },
            mineru: MineruConfig {
                source: "https://github.com/opendatalab/MinerU".into(),
                binary: services_dir.join("mineru/.venv/bin/mineru-kit"),
                server_binary: services_dir.join("mineru/.venv/bin/mineru-kit"),
                server_workdir: services_dir.join("mineru"),
                server_args: vec![
                    "api-server".into(),
                    "--host".into(),
                    "127.0.0.1".into(),
                    "--port".into(),
                    "8000".into(),
                    "--tier".into(),
                    "standard".into(),
                ],
                threads: 1,
            },
        }
    }
}

impl Config {
    /// Check a file name against configured suffixes, matching without case sensitivity.
    pub fn accepts(&self, path: &Path) -> bool {
        let name = path.file_name().and_then(|s| s.to_str());
        name.is_some_and(|name| {
            self.scanendfilter.iter().any(|suffix| {
                !suffix.is_empty() && name.to_ascii_lowercase().ends_with(&suffix.to_ascii_lowercase())
            })
        })
    }

    /// Extensions used with fsscanner's single-extension scan operations.
    /// Repeated extensions and dotted/undotted spellings are normalized.
    pub fn scan_extensions(&self) -> Result<Vec<String>> {
        let mut extensions = Vec::new();
        for suffix in &self.scanendfilter {
            let Some(extension) = suffix.strip_prefix('.') else {
                return Err(crate::error::Error::InvalidPath("scanendfilter entries must start with '.'"));
            };
            if extension.is_empty() || extension.contains('.') || extension.contains('/') || extension.contains('\\') {
                return Err(crate::error::Error::InvalidPath("scanendfilter must contain simple file extensions"));
            }
            let extension = extension.to_ascii_lowercase();
            if !extensions.contains(&extension) {
                extensions.push(extension);
            }
        }
        if extensions.is_empty() {
            return Err(crate::error::Error::InvalidPath("scanendfilter must not be empty"));
        }
        Ok(extensions)
    }

    pub fn load(path: &std::path::Path) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        Ok(fs::write(path, content)?)
    }

    /// Returns `true` if the configuration file was created.
    pub fn ensure_exist(path: &std::path::Path) -> Result<bool> {
        if path.exists() {
            return Ok(false);
        }

        Self::default().save(path)?;
        Ok(true)
    }

    pub fn load_or_create() -> Result<Self> {
        let path = Self::default_path();

        if Self::ensure_exist(&path)? {
            println!("Created default config at {:?}", path);
            Ok(Self::default())
        } else {
            Self::load(&path)
        }
    }

    pub fn default_path() -> PathBuf {
        if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(config_home).join("pdf_to_text/config.json");
        }

        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(".config/pdf_to_text/config.json");
        }

        PathBuf::from("config.json")
    }
}

//! A JSON-file plugin registry.
//!
//! # Storage decision
//!
//! The SRS deliberately leaves storage unspecified. A single JSON document
//! under the crawl home directory is chosen because the data is small, must be
//! inspectable by hand, and needs no concurrent-writer story: `crawl plugin`
//! commands are interactive and serial.
//!
//! Installation records the *validated manifest plus its source directory*; it
//! does not copy plugin files. That answers OQ-024 conservatively: `remove`
//! unregisters and never deletes an author's code or Python environment.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crawl_application::ports::registry::{PluginRegistry, RegistryError};
use crawl_domain::ids::PluginId;
use crawl_domain::plugin::Plugin;
use serde::{Deserialize, Serialize};

/// Environment variable that relocates the registry, chiefly for tests.
pub const CRAWL_HOME_ENV: &str = "CRAWL_HOME";

/// On-disk registry document.
#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryDocument {
    /// Schema version of this document, distinct from the plugin API version.
    version: u32,
    /// Registered plugins, keyed by name so lookup is O(log n) and stable.
    plugins: BTreeMap<String, Plugin>,
}

/// Plugin registry backed by one JSON file.
#[derive(Debug, Clone)]
pub struct FileRegistry {
    path: PathBuf,
}

impl FileRegistry {
    /// Current on-disk document version.
    pub const DOCUMENT_VERSION: u32 = 1;

    /// Opens a registry at an explicit path.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Opens the registry at the default location.
    ///
    /// Resolution order is `$CRAWL_HOME/registry.json`, then
    /// `$XDG_DATA_HOME/crawl/registry.json`, then `~/.local/share/crawl`, then
    /// the current directory as a last resort.
    #[must_use]
    pub fn default_location() -> Self {
        let home = std::env::var_os(CRAWL_HOME_ENV)
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_DATA_HOME").map(|base| PathBuf::from(base).join("crawl"))
            })
            .or_else(|| {
                std::env::var_os("HOME").map(|base| PathBuf::from(base).join(".local/share/crawl"))
            })
            .unwrap_or_else(|| PathBuf::from(".crawl"));
        Self::at(home.join("registry.json"))
    }

    /// Returns the path of the registry document.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the registry document.
    ///
    /// A missing file is an empty registry, not an error: a first run has
    /// nothing installed, and treating that as a failure would make every
    /// command fail until something was installed. A file that exists but
    /// cannot be parsed *is* an error, because silently discarding somebody's
    /// installed plugins would be worse than refusing to continue.
    fn read(&self) -> Result<RegistryDocument, RegistryError> {
        match fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| {
                RegistryError::Corrupt(format!("{}: {error}", self.path.display()))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(RegistryDocument {
                version: Self::DOCUMENT_VERSION,
                plugins: BTreeMap::new(),
            }),
            Err(error) => Err(RegistryError::Unavailable(format!(
                "{}: {error}",
                self.path.display()
            ))),
        }
    }

    /// Writes the document atomically: a crash mid-write leaves the previous
    /// registry intact rather than a truncated file.
    fn write(&self, document: &RegistryDocument) -> Result<(), RegistryError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                RegistryError::Unavailable(format!("{}: {error}", parent.display()))
            })?;
        }
        let serialized = serde_json::to_string_pretty(document)
            .map_err(|error| RegistryError::Unavailable(error.to_string()))?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, serialized).map_err(|error| {
            RegistryError::Unavailable(format!("{}: {error}", temporary.display()))
        })?;
        fs::rename(&temporary, &self.path).map_err(|error| {
            RegistryError::Unavailable(format!("{}: {error}", self.path.display()))
        })
    }
}

impl PluginRegistry for FileRegistry {
    fn find(&self, id: &PluginId) -> Result<Option<Plugin>, RegistryError> {
        Ok(self.read()?.plugins.get(id.as_str()).cloned())
    }

    fn list(&self) -> Result<Vec<Plugin>, RegistryError> {
        Ok(self.read()?.plugins.into_values().collect())
    }

    fn install(&self, plugin: &Plugin, replace: bool) -> Result<(), RegistryError> {
        let mut document = self.read()?;
        document.version = Self::DOCUMENT_VERSION;
        let key = plugin.name.as_str().to_owned();
        if document.plugins.contains_key(&key) && !replace {
            return Err(RegistryError::AlreadyInstalled(plugin.id()));
        }
        document.plugins.insert(key, plugin.clone());
        self.write(&document)
    }

    fn remove(&self, id: &PluginId) -> Result<(), RegistryError> {
        let mut document = self.read()?;
        if document.plugins.remove(id.as_str()).is_none() {
            return Err(RegistryError::NotFound(id.clone()));
        }
        self.write(&document)
    }
}

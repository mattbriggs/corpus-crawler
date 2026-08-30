//! Plugin registry repository port (SRS 6.5).

use crawl_domain::ids::PluginId;
use crawl_domain::plugin::Plugin;

/// Failure to read or write registered plugin metadata.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// The registry store could not be read or written.
    #[error("plugin registry unavailable: {0}")]
    Unavailable(String),
    /// The registry contents could not be interpreted.
    #[error("plugin registry is corrupt: {0}")]
    Corrupt(String),
    /// The named plugin is not registered.
    #[error("plugin {0} is not installed")]
    NotFound(PluginId),
    /// A plugin with this name is already registered (REQ-011).
    #[error("plugin {0} is already installed; pass --force to replace it")]
    AlreadyInstalled(PluginId),
}

/// Persistence of registered plugin metadata.
///
/// Storage technology is deliberately unspecified by the SRS, so this is a
/// repository rather than a concrete file API.
pub trait PluginRegistry: Send + Sync {
    /// Returns the registered plugin with this identity, if any.
    ///
    /// # Errors
    /// Returns [`RegistryError`] when the store cannot be read.
    fn find(&self, id: &PluginId) -> Result<Option<Plugin>, RegistryError>;

    /// Returns every registered plugin, ordered by name.
    ///
    /// # Errors
    /// Returns [`RegistryError`] when the store cannot be read.
    fn list(&self) -> Result<Vec<Plugin>, RegistryError>;

    /// Creates or replaces the registration for a plugin.
    ///
    /// # Errors
    /// Returns [`RegistryError::AlreadyInstalled`] when `replace` is false and
    /// the name is taken, or [`RegistryError::Unavailable`] on write failure.
    fn install(&self, plugin: &Plugin, replace: bool) -> Result<(), RegistryError>;

    /// Removes a registration (REQ-022).
    ///
    /// Only the registry association is removed; plugin files and Python
    /// environments are left alone (OQ-024).
    ///
    /// # Errors
    /// Returns [`RegistryError::NotFound`] when nothing was registered.
    fn remove(&self, id: &PluginId) -> Result<(), RegistryError>;
}

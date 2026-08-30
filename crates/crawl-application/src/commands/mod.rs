//! Application commands (SRS 6.6). Each is a thin use case over the ports.

use crawl_domain::ids::PluginId;
use crawl_domain::plugin::Plugin;

use crate::ports::registry::{PluginRegistry, RegistryError};

/// Registers a validated plugin (REQ-010, REQ-011).
///
/// # Errors
/// Returns [`RegistryError`] when the store cannot be written or the name is
/// taken and `replace` is false.
pub fn install_plugin(
    registry: &dyn PluginRegistry,
    plugin: &Plugin,
    replace: bool,
) -> Result<(), RegistryError> {
    registry.install(plugin, replace)
}

/// Lists registered plugins (REQ-020).
///
/// # Errors
/// Returns [`RegistryError`] when the store cannot be read.
pub fn list_plugins(registry: &dyn PluginRegistry) -> Result<Vec<Plugin>, RegistryError> {
    registry.list()
}

/// Resolves one registered plugin (REQ-021, REQ-031).
///
/// # Errors
/// Returns [`RegistryError::NotFound`] when the plugin is not registered.
pub fn resolve_plugin(
    registry: &dyn PluginRegistry,
    id: &PluginId,
) -> Result<Plugin, RegistryError> {
    registry
        .find(id)?
        .ok_or_else(|| RegistryError::NotFound(id.clone()))
}

/// Removes a registration (REQ-022).
///
/// # Errors
/// Returns [`RegistryError::NotFound`] when nothing was registered.
pub fn remove_plugin(registry: &dyn PluginRegistry, id: &PluginId) -> Result<(), RegistryError> {
    registry.remove(id)
}

//! `crawl plugin ...` (REQ-010 - REQ-022).

use std::path::Path;

use crawl_application::commands as use_cases;
use crawl_application::ports::registry::PluginRegistry;
use crawl_application::services::plugin_validation::validate_plugin;
use crawl_domain::ids::{PluginId, PluginName};
use crawl_domain::manifest::PluginManifest;
use crawl_domain::plugin::Plugin;
use crawl_infrastructure::ProcessWorkerFactory;

use crate::exit_codes;

/// Loads and validates a manifest from a directory or file path.
///
/// # Errors
/// Returns a human-readable message and the configuration exit code.
pub fn load_plugin(source: &Path) -> Result<Plugin, (String, i32)> {
    let (manifest, manifest_path) = PluginManifest::load(source)
        .map_err(|error| (error.to_string(), exit_codes::CONFIGURATION_FAILURE))?;
    let root = manifest_path
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), Path::to_path_buf);
    let root = root.canonicalize().unwrap_or(root);
    manifest
        .validate(&root)
        .map_err(|error| (error.to_string(), exit_codes::CONFIGURATION_FAILURE))
}

/// Parses a registry identity from user input.
///
/// # Errors
/// Returns a message when the name is not a legal plugin name.
pub fn parse_id(name: &str) -> Result<PluginId, (String, i32)> {
    PluginName::new(name)
        .map(PluginId::new)
        .map_err(|error| (error.to_string(), exit_codes::CONFIGURATION_FAILURE))
}

/// `crawl plugin install`.
pub async fn install(
    registry: &dyn PluginRegistry,
    source: &Path,
    force: bool,
    skip_runtime_check: bool,
) -> Result<i32, (String, i32)> {
    let plugin = load_plugin(source)?;

    if !skip_runtime_check {
        let factory = ProcessWorkerFactory;
        validate_plugin(&plugin, &factory)
            .await
            .map_err(|error| (error.to_string(), exit_codes::PLUGIN_FAILURE))?;
    }

    use_cases::install_plugin(registry, &plugin, force)
        .map_err(|error| (error.to_string(), exit_codes::CONFIGURATION_FAILURE))?;

    println!(
        "installed {} ({} extensions, {} schema fields)",
        plugin.display_identity(),
        plugin.input.extensions().count(),
        plugin.schema.len()
    );
    Ok(exit_codes::SUCCESS)
}

/// `crawl plugin validate`.
pub async fn validate(
    registry: &dyn PluginRegistry,
    target: &str,
    as_path: bool,
) -> Result<i32, (String, i32)> {
    let plugin = if as_path {
        load_plugin(Path::new(target))?
    } else {
        let id = parse_id(target)?;
        use_cases::resolve_plugin(registry, &id)
            .map_err(|error| (error.to_string(), exit_codes::CONFIGURATION_FAILURE))?
    };

    println!("manifest:   ok ({})", plugin.display_identity());

    let factory = ProcessWorkerFactory;
    let report = validate_plugin(&plugin, &factory)
        .await
        .map_err(|error| (error.to_string(), exit_codes::PLUGIN_FAILURE))?;

    println!("runtime:    ok ({})", plugin.runtime.command.join(" "));
    println!("entrypoint: ok");
    match report.selftest {
        None => println!("self-test:  not declared"),
        Some(Ok(message)) => println!("self-test:  ok ({message})"),
        Some(Err(message)) => {
            return Err((
                format!("self-test failed: {message}"),
                exit_codes::PLUGIN_FAILURE,
            ))
        }
    }
    Ok(exit_codes::SUCCESS)
}

/// `crawl plugin list`.
pub fn list(registry: &dyn PluginRegistry, json: bool) -> Result<i32, (String, i32)> {
    let plugins = use_cases::list_plugins(registry)
        .map_err(|error| (error.to_string(), exit_codes::CONFIGURATION_FAILURE))?;

    if json {
        let rendered = serde_json::to_string_pretty(&plugins)
            .map_err(|error| (error.to_string(), exit_codes::HOST_FAILURE))?;
        println!("{rendered}");
        return Ok(exit_codes::SUCCESS);
    }

    if plugins.is_empty() {
        println!("no plugins installed");
        return Ok(exit_codes::SUCCESS);
    }

    let width = plugins
        .iter()
        .map(|plugin| plugin.name.as_str().len())
        .max()
        .unwrap_or(4)
        .max(4);
    println!("{:<width$}  {:<10}  EXTENSIONS", "NAME", "VERSION");
    for plugin in &plugins {
        println!(
            "{:<width$}  {:<10}  {}",
            plugin.name.as_str(),
            plugin.version.as_str(),
            plugin.input.extensions().collect::<Vec<_>>().join(" ")
        );
    }
    Ok(exit_codes::SUCCESS)
}

/// `crawl plugin inspect`.
pub fn inspect(
    registry: &dyn PluginRegistry,
    name: &str,
    json: bool,
) -> Result<i32, (String, i32)> {
    let id = parse_id(name)?;
    let plugin = use_cases::resolve_plugin(registry, &id)
        .map_err(|error| (error.to_string(), exit_codes::CONFIGURATION_FAILURE))?;
    let rendered = if json {
        serde_json::to_string_pretty(&plugin)
            .map_err(|error| (error.to_string(), exit_codes::HOST_FAILURE))?
    } else {
        serde_yaml::to_string(&plugin)
            .map_err(|error| (error.to_string(), exit_codes::HOST_FAILURE))?
    };
    println!("{rendered}");
    Ok(exit_codes::SUCCESS)
}

/// `crawl plugin remove`.
pub fn remove(registry: &dyn PluginRegistry, name: &str) -> Result<i32, (String, i32)> {
    let id = parse_id(name)?;
    use_cases::remove_plugin(registry, &id)
        .map_err(|error| (error.to_string(), exit_codes::CONFIGURATION_FAILURE))?;
    println!("removed {id}");
    Ok(exit_codes::SUCCESS)
}

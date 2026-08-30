//! The validated plugin aggregate.
//!
//! A [`Plugin`] is what the rest of the system consumes. It can only be
//! produced by validating a [`crate::manifest::PluginManifest`], so anything
//! holding a `Plugin` knows the API version is supported, the runtime command
//! is launchable, the extensions are well formed, and the output schema is
//! internally consistent (SRS 6.2).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ids::{ApiVersion, PluginId, PluginName, PluginVersion};
use crate::policy::{QueueCapacity, RetryLimit, Timeout, WorkerCount};
use crate::schema::{ExtraFieldPolicy, OutputSchema};

/// How to launch a worker process for this plugin (REQ-152, VAL-005, VAL-006).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSpec {
    /// The runtime family. Only `python` is supported in v1.
    pub runtime_type: RuntimeType,
    /// Argument vector; the first element is the program to execute.
    pub command: Vec<String>,
    /// Working directory for the worker, relative to the plugin root when
    /// relative. Defaults to the plugin root.
    pub working_directory: Option<PathBuf>,
    /// Extra environment variables passed to the worker process.
    pub environment: Vec<(String, String)>,
    /// Whether the plugin implements the optional `selftest` control request
    /// (REQ-019, OQ-023).
    pub selftest: bool,
}

impl RuntimeSpec {
    /// Returns the program to execute.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.command[0]
    }

    /// Returns the arguments following the program.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.command[1..]
    }
}

/// Supported worker runtime families (VAL-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeType {
    /// A Python worker speaking the JSONL protocol over stdin/stdout.
    Python,
}

/// Which files a plugin accepts (REQ-153, VAL-007).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSpec {
    /// Normalized, lowercase, dot-prefixed extensions.
    extensions: BTreeSet<String>,
    /// Whether traversal follows symbolic links (REQ-044).
    pub follow_symlinks: bool,
}

impl InputSpec {
    /// Creates an input specification from already-normalized extensions.
    ///
    /// # Caller contract
    ///
    /// Extensions must have been validated and normalized by
    /// [`crate::manifest::normalize_extension`]; this constructor does not
    /// re-validate them.
    #[must_use]
    pub fn new(extensions: BTreeSet<String>, follow_symlinks: bool) -> Self {
        Self {
            extensions,
            follow_symlinks,
        }
    }

    /// Returns the normalized extensions in sorted order.
    pub fn extensions(&self) -> impl Iterator<Item = &str> {
        self.extensions.iter().map(String::as_str)
    }

    /// Returns `true` when the path's extension matches this specification.
    ///
    /// # Matching rule
    ///
    /// Matching is case-insensitive and compares only the final extension, so
    /// `README.MD` matches `.md` and `archive.tar.gz` matches `.gz` but not
    /// `.tar.gz`. A file with no extension never matches, which is why
    /// dotfiles such as `.gitignore` are treated as extensionless rather than
    /// as an extension of `.gitignore`.
    #[must_use]
    pub fn matches(&self, path: &Path) -> bool {
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            return false;
        };
        let normalized = format!(".{}", extension.to_ascii_lowercase());
        self.extensions.contains(&normalized)
    }
}

/// Manifest-declared execution defaults (REQ-157).
///
/// Every field is optional: an absent value falls through to the host default
/// during effective-configuration resolution (REQ-158).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ExecutionDefaults {
    /// Preferred worker count.
    pub workers: Option<WorkerCount>,
    /// Preferred per-file timeout.
    pub timeout: Option<Timeout>,
    /// Preferred retry allowance.
    pub max_retries: Option<RetryLimit>,
    /// Preferred bounded queue depth.
    pub queue_capacity: Option<QueueCapacity>,
}

/// A validated, registrable plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plugin {
    /// Supported API version declared by the manifest.
    pub api_version: ApiVersion,
    /// Registry name.
    pub name: PluginName,
    /// Plugin version.
    pub version: PluginVersion,
    /// Optional human description.
    pub description: Option<String>,
    /// How to launch a worker.
    pub runtime: RuntimeSpec,
    /// Which files the plugin accepts.
    pub input: InputSpec,
    /// Declared output schema.
    pub schema: OutputSchema,
    /// Policy for undeclared record fields.
    pub extra_fields: ExtraFieldPolicy,
    /// Manifest-declared execution defaults.
    pub execution: ExecutionDefaults,
    /// Absolute path of the directory containing `plugin.yaml`.
    ///
    /// Worker commands are launched with this as their working directory, so a
    /// manifest can name its entry point relative to the plugin root.
    pub root: PathBuf,
}

impl Plugin {
    /// Returns the registry identity of this plugin.
    #[must_use]
    pub fn id(&self) -> PluginId {
        PluginId::new(self.name.clone())
    }

    /// Returns the working directory for worker processes.
    #[must_use]
    pub fn working_directory(&self) -> PathBuf {
        match &self.runtime.working_directory {
            Some(directory) if directory.is_absolute() => directory.clone(),
            Some(directory) => self.root.join(directory),
            None => self.root.clone(),
        }
    }

    /// Returns a one-line identity string for logs and CLI output.
    #[must_use]
    pub fn display_identity(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(extensions: &[&str]) -> InputSpec {
        InputSpec::new(extensions.iter().map(|e| (*e).to_owned()).collect(), false)
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        let input = spec(&[".txt", ".md"]);
        assert!(input.matches(Path::new("/a/b.txt")));
        assert!(input.matches(Path::new("/a/README.MD")));
        assert!(input.matches(Path::new("/a/b.TxT")));
        assert!(!input.matches(Path::new("/a/b.rs")));
    }

    #[test]
    fn only_the_final_extension_is_considered() {
        let input = spec(&[".gz"]);
        assert!(input.matches(Path::new("/a/archive.tar.gz")));
        assert!(!spec(&[".tar.gz"]).matches(Path::new("/a/archive.tar.gz")));
    }

    #[test]
    fn extensionless_files_never_match() {
        let input = spec(&[".txt"]);
        assert!(!input.matches(Path::new("/a/LICENSE")));
        assert!(!input.matches(Path::new("/a/.gitignore")));
    }
}

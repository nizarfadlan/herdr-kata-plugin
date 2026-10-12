//! Plugin configuration, loaded from `HERDR_PLUGIN_CONFIG_DIR/config.toml`.
//! Missing file: defaults. Unknown keys are ignored so older configs keep
//! working when new keys are added.

use std::path::Path;

use serde::Deserialize;

use crate::constants::{CONFIG_FILE, DEFAULT_NAMESPACE, DEFAULT_SOCKET, KATA_RUNTIME};
use crate::errors::{ErrorKind, PluginError, Result};

/// User-editable configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PluginConfig {
    /// Agent adapter id: `codex` (default), `opencode`, or a custom agent.
    pub agent: String,
    /// Containerd CLI used by the Kata driver (`ctr` or `nerdctl`).
    pub kata_bin: String,
    /// Path of the containerd socket the driver talks to.
    pub containerd_socket: String,
    /// Containerd namespace for this plugin's microVMs.
    pub containerd_namespace: String,
    /// Runtime name registered in containerd for Kata.
    pub kata_runtime: String,
    /// Guest image override (empty: per-agent default from the profile).
    pub image: String,
    /// Directory holding the VM profiles, relative to the plugin root.
    pub profiles_dir: String,
    /// Extra arguments appended to the agent command inside the microVM.
    pub agent_args: Vec<String>,
    /// Host directory mounted read-only as the agent's config inside the
    /// microVM (the profile's `{agent_config}` template). Empty: the mount
    /// is skipped. Example: `~/.codex` (expanded) for Codex.
    pub agent_config_dir: String,
}

impl Default for PluginConfig {
    fn default() -> Self {
        PluginConfig {
            agent: "codex".to_string(),
            kata_bin: "ctr".to_string(),
            containerd_socket: DEFAULT_SOCKET.to_string(),
            containerd_namespace: DEFAULT_NAMESPACE.to_string(),
            kata_runtime: KATA_RUNTIME.to_string(),
            image: String::new(),
            profiles_dir: "profiles".to_string(),
            agent_args: Vec::new(),
            agent_config_dir: String::new(),
        }
    }
}

/// Loads the config, falling back to defaults when the file is absent.
pub fn load_config(config_dir: &Path) -> Result<PluginConfig> {
    let path = config_dir.join(CONFIG_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PluginConfig::default())
        }
        Err(error) => {
            return Err(PluginError::new(
                ErrorKind::Config,
                format!("cannot read {}: {error}", path.display()),
            ))
        }
    };
    toml::from_str(&text).map_err(|error| {
        PluginError::new(
            ErrorKind::Config,
            format!("cannot parse {}: {error}", path.display()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_select_codex_and_ctr() {
        let config = PluginConfig::default();
        assert_eq!(config.agent, "codex");
        assert_eq!(config.kata_bin, "ctr");
        assert_eq!(config.kata_runtime, KATA_RUNTIME);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = load_config(dir.path()).expect("defaults");
        assert_eq!(config.agent, "codex");
    }

    #[test]
    fn partial_file_merges_over_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(CONFIG_FILE), "agent = \"opencode\"\n").expect("write");
        let config = load_config(dir.path()).expect("parsed");
        assert_eq!(config.agent, "opencode");
        assert_eq!(config.kata_bin, "ctr");
    }

    #[test]
    fn broken_file_is_an_error_not_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(CONFIG_FILE), "agent = [").expect("write");
        let error = load_config(dir.path()).expect_err("must fail");
        assert_eq!(error.kind, ErrorKind::Config);
    }
}

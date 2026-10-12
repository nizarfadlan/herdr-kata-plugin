//! Plugin-wide constants: ids, action names, key chords, environment names.

/// The plugin id, as declared in `herdr-plugin.toml`.
pub const PLUGIN_ID: &str = "kata.microvm";
/// The binary name installed under `bin/`.
pub const BINARY_NAME: &str = "herdr-kata";
/// The GitHub repository install shorthand.
pub const REPO: &str = "nizarfadlan/herdr-kata-plugin";

/// The result line an action's caller parses.
pub const RESULT_PREFIX: &str = "HERDR_SANDBOX_RESULT: ";
/// Schema version of the result line.
pub const RESULT_SCHEMA_VERSION: u32 = 1;

/// Action ids, in manifest order.
pub const ACTIONS: [&str; 12] = [
    "doctor",
    "install-keybindings",
    "start-agent",
    "reconnect",
    "open-shell",
    "stop",
    "info",
    "verify-microvm",
    "prune-mappings",
    "microvms",
    "list-microvms",
    "forget-mapping",
];

/// Key chords installed by the `install-keybindings` action.
pub const KEYBINDINGS: [(&str, &str); 4] = [
    ("prefix+Shift+a", "start-agent"),
    ("prefix+Shift+o", "microvms"),
    ("prefix+Shift+b", "reconnect"),
    ("prefix+Shift+s", "open-shell"),
];

/// Kata runtime name registered in containerd (shim `containerd-shim-kata-v2`).
pub const KATA_RUNTIME: &str = "io.containerd.kata.v2";
/// Default containerd namespace for this plugin's microVMs.
pub const DEFAULT_NAMESPACE: &str = "herdr-kata";
/// Default path of the containerd socket.
pub const DEFAULT_SOCKET: &str = "/run/containerd/containerd.sock";

/// Config file name inside `HERDR_PLUGIN_CONFIG_DIR`.
pub const CONFIG_FILE: &str = "config.toml";
/// State file name inside `HERDR_PLUGIN_STATE_DIR`.
pub const STATE_FILE: &str = "mappings.json";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_match_the_manifest() {
        // Keep this in sync with herdr-plugin.toml.
        assert_eq!(ACTIONS.len(), 12);
        assert!(ACTIONS.contains(&"doctor"));
        assert!(ACTIONS.contains(&"microvms"));
    }
}

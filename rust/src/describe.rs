//! Machine-readable self-description: action ids, config keys, constants.

use serde_json::{json, Value};

use crate::agents::default_agent;
use crate::constants::{ACTIONS, KATA_RUNTIME, KEYBINDINGS, PLUGIN_ID};

/// The `describe` payload.
pub fn describe() -> Value {
    json!({
        "plugin": PLUGIN_ID,
        "version": env!("CARGO_PKG_VERSION"),
        "actions": ACTIONS,
        "keybindings": KEYBINDINGS
            .iter()
            .map(|(key, action)| json!({"key": key, "action": action}))
            .collect::<Vec<_>>(),
        "configKeys": [
            "agent",
            "kata_bin",
            "containerd_socket",
            "containerd_namespace",
            "kata_runtime",
            "image",
            "profiles_dir",
            "agent_args",
        ],
        "defaultAgent": default_agent(),
        "kataRuntime": KATA_RUNTIME,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_is_valid_json_with_actions() {
        let value = describe();
        let actions = value["actions"].as_array().expect("actions is an array");
        assert!(actions.iter().any(|a| a == "doctor"));
        assert_eq!(value["plugin"], PLUGIN_ID);
    }
}

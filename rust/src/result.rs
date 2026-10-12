//! The `HERDR_SANDBOX_RESULT:` line an action's caller parses.

use serde_json::json;

use crate::constants::{PLUGIN_ID, RESULT_PREFIX, RESULT_SCHEMA_VERSION};
use crate::errors::ErrorKind;

/// Prints the result line for an action invocation.
pub fn print_result(action: &str, ok: bool, kind: Option<ErrorKind>, message: Option<&str>) {
    let mut value = json!({
        "schemaVersion": RESULT_SCHEMA_VERSION,
        "plugin": PLUGIN_ID,
        "action": action,
        "ok": ok,
    });
    if let Some(kind) = kind {
        value["errorKind"] = json!(kind.as_str());
    }
    if let Some(message) = message {
        value["message"] = json!(message);
    }
    println!("{RESULT_PREFIX}{value}");
}

/// Prints an ok result.
pub fn ok(action: &str) {
    print_result(action, true, None, None);
}

/// Prints a failure result from a plugin error.
pub fn err(action: &str, error: &crate::errors::PluginError) {
    print_result(action, false, Some(error.kind), Some(&error.message));
    if !error.output.trim().is_empty() {
        eprintln!("{}", error.output.trim());
    }
}

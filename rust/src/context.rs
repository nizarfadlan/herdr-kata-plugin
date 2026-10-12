//! The runtime environment Herdr injects into every plugin command.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// Snapshot of the process environment relevant to the plugin.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub vars: HashMap<String, String>,
}

impl Env {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }

    pub fn plugin_id(&self) -> &str {
        self.get("HERDR_PLUGIN_ID").unwrap_or("kata.microvm")
    }
    pub fn plugin_root(&self) -> Option<&str> {
        self.get("HERDR_PLUGIN_ROOT")
    }
    pub fn config_dir(&self) -> Option<&str> {
        self.get("HERDR_PLUGIN_CONFIG_DIR")
    }
    pub fn state_dir(&self) -> Option<&str> {
        self.get("HERDR_PLUGIN_STATE_DIR")
    }
    pub fn action_id(&self) -> Option<&str> {
        self.get("HERDR_PLUGIN_ACTION_ID")
    }
    pub fn event(&self) -> Option<&str> {
        self.get("HERDR_PLUGIN_EVENT")
    }
    pub fn event_json(&self) -> Option<&str> {
        self.get("HERDR_PLUGIN_EVENT_JSON")
    }
    pub fn pane_id(&self) -> Option<&str> {
        self.get("HERDR_PANE_ID")
    }
    pub fn workspace_id(&self) -> Option<&str> {
        self.get("HERDR_WORKSPACE_ID")
    }
    pub fn herdr_bin(&self) -> &str {
        self.get("HERDR_BIN_PATH").unwrap_or("herdr")
    }
    pub fn inside_herdr(&self) -> bool {
        self.get("HERDR_ENV") == Some("1")
    }
}

/// Reads the current process environment.
pub fn process_env() -> Env {
    Env {
        vars: std::env::vars().collect(),
    }
}

/// Parses `HERDR_PLUGIN_CONTEXT_JSON` into an object.
pub fn context_object(env: &Env) -> Option<Map<String, Value>> {
    let raw = env.get("HERDR_PLUGIN_CONTEXT_JSON")?;
    serde_json::from_str::<Value>(raw)
        .ok()?
        .as_object()
        .cloned()
}

fn non_empty_string<'a>(context: &'a Map<String, Value>, path: &[&str]) -> Option<&'a str> {
    let mut current = context.get(*path.first()?)?;
    for key in &path[1..] {
        current = current.get(*key)?;
    }
    current.as_str().filter(|text| !text.is_empty())
}

/// Canonical absolute form of a path: symlinks resolved when the path exists.
pub fn canonical_path(value: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(value) {
        return real;
    }
    match (value.parent(), value.file_name()) {
        (Some(parent), Some(name)) => canonical_path(parent).join(name),
        _ => value.to_path_buf(),
    }
}

/// Returns the git top-level directory containing `dir`, or `None`.
pub fn git_toplevel(dir: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-C", dir, "rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let top = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!top.is_empty()).then_some(top)
}

/// Picks the directory the microVM grants read-write (the workspace root).
/// Order: the workspace's worktree checkout, the workspace directory, the git
/// top level of the focused pane's directory, then that directory itself.
pub fn resolve_workspace_root(context: &Map<String, Value>) -> Option<PathBuf> {
    if let Some(checkout) = non_empty_string(context, &["worktree", "checkout_path"]) {
        return Some(canonical_path(Path::new(checkout)));
    }
    if let Some(workspace) = non_empty_string(context, &["workspace_cwd"]) {
        return Some(canonical_path(Path::new(workspace)));
    }
    let pane_cwd = non_empty_string(context, &["focused_pane_cwd"])?;
    Some(canonical_path(Path::new(
        git_toplevel(pane_cwd).as_deref().unwrap_or(pane_cwd),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn workspace_root_prefers_the_worktree_checkout() {
        let context = object(json!({
            "worktree": {"checkout_path": "/wt"},
            "workspace_cwd": "/ws",
            "focused_pane_cwd": "/ws/src",
        }));
        // /wt does not exist here, so it stays unresolved but absolute.
        assert_eq!(
            resolve_workspace_root(&context).expect("root"),
            PathBuf::from("/wt")
        );
    }

    #[test]
    fn workspace_root_falls_back_to_pane_cwd() {
        let context = object(json!({"focused_pane_cwd": "/tmp"}));
        assert_eq!(
            resolve_workspace_root(&context).expect("root"),
            std::fs::canonicalize("/tmp").expect("canonicalize")
        );
    }

    #[test]
    fn empty_context_resolves_to_nothing() {
        let context = object(json!({}));
        assert!(resolve_workspace_root(&context).is_none());
    }
}

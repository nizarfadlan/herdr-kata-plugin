//! Pane → microVM mappings, persisted as JSON in `HERDR_PLUGIN_STATE_DIR`.
//! Writes are atomic (temp file + rename).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::constants::STATE_FILE;
use crate::errors::{ErrorKind, PluginError, Result};

/// One tracked microVM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapping {
    pub pane_id: String,
    pub microvm_id: String,
    pub agent: String,
    pub profile: String,
    pub worktree: String,
    pub created_at_secs: u64,
}

/// The persisted state file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub mappings: Vec<Mapping>,
}

fn state_path(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join(STATE_FILE)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Loads the state; a missing file means "no mappings yet".
pub fn load(state_dir: &Path) -> Result<State> {
    let path = state_path(state_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
        Err(error) => {
            return Err(PluginError::new(
                ErrorKind::Startup,
                format!("cannot read {}: {error}", path.display()),
            ))
        }
    };
    serde_json::from_str(&text).map_err(|error| {
        PluginError::new(
            ErrorKind::Startup,
            format!("cannot parse {}: {error}", path.display()),
        )
    })
}

/// Saves the state atomically.
pub fn save(state_dir: &Path, state: &State) -> Result<()> {
    std::fs::create_dir_all(state_dir).map_err(|error| {
        PluginError::new(
            ErrorKind::Startup,
            format!("cannot create {}: {error}", state_dir.display()),
        )
    })?;
    let path = state_path(state_dir);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(state).map_err(|error| {
        PluginError::new(
            ErrorKind::Startup,
            format!("cannot serialise state: {error}"),
        )
    })?;
    std::fs::write(&tmp, text).map_err(|error| {
        PluginError::new(
            ErrorKind::Startup,
            format!("cannot write {}: {error}", tmp.display()),
        )
    })?;
    std::fs::rename(&tmp, &path).map_err(|error| {
        PluginError::new(
            ErrorKind::Startup,
            format!("cannot rename {}: {error}", path.display()),
        )
    })
}

impl State {
    /// Records a mapping, replacing any previous one for the pane.
    pub fn upsert(&mut self, mapping: Mapping) {
        self.mappings.retain(|m| m.pane_id != mapping.pane_id);
        self.mappings.push(mapping);
    }

    /// Drops the mapping for a pane; `true` when one existed.
    pub fn remove_pane(&mut self, pane_id: &str) -> bool {
        let before = self.mappings.len();
        self.mappings.retain(|m| m.pane_id != pane_id);
        self.mappings.len() != before
    }

    /// Finds the mapping for a pane.
    pub fn by_pane(&self, pane_id: &str) -> Option<&Mapping> {
        self.mappings.iter().find(|m| m.pane_id == pane_id)
    }
}

/// Builds a fresh mapping with the current timestamp.
pub fn new_mapping(
    pane_id: &str,
    microvm_id: &str,
    agent: &str,
    profile: &str,
    worktree: &str,
) -> Mapping {
    Mapping {
        pane_id: pane_id.to_string(),
        microvm_id: microvm_id.to_string(),
        agent: agent.to_string(),
        profile: profile.to_string(),
        worktree: worktree.to_string(),
        created_at_secs: now_secs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_the_state_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = load(dir.path()).expect("empty at first");
        assert!(state.mappings.is_empty());
        state.upsert(new_mapping(
            "p1",
            "kata-p1-abc123",
            "codex",
            "prof",
            "/work",
        ));
        save(dir.path(), &state).expect("save");
        let reloaded = load(dir.path()).expect("reload");
        assert_eq!(reloaded.mappings.len(), 1);
        assert_eq!(
            reloaded.by_pane("p1").expect("found").microvm_id,
            "kata-p1-abc123"
        );
        let mut state = reloaded;
        assert!(state.remove_pane("p1"));
        assert!(!state.remove_pane("p1"));
    }

    #[test]
    fn upsert_replaces_the_panes_previous_mapping() {
        let mut state = State::default();
        state.upsert(new_mapping("p1", "old", "codex", "prof", "/work"));
        state.upsert(new_mapping("p1", "new", "codex", "prof", "/work"));
        assert_eq!(state.mappings.len(), 1);
        assert_eq!(state.by_pane("p1").expect("found").microvm_id, "new");
    }
}

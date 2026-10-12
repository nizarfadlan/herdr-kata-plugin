//! The overlay's data model: one row per tracked pane-to-microVM mapping,
//! enriched with the live container state from containerd.

use std::path::Path;

use crate::kata::CtrBackend;
use crate::state::{self, Mapping};

/// One microVM row in the overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub mapping: Mapping,
    /// Whether the container exists in the plugin's containerd namespace.
    pub live: bool,
    /// The container's runtime name, when live.
    pub runtime: Option<String>,
}

/// Loads one row per tracked mapping; a missing state file means no rows.
pub fn load_rows(state_dir: &Path, backend: &CtrBackend) -> Vec<Row> {
    let state = state::load(state_dir).unwrap_or_default();
    state
        .mappings
        .into_iter()
        .map(|mapping| {
            let (live, runtime) = match backend.describe(&mapping.microvm_id) {
                Ok(vm) => (true, vm.runtime),
                Err(_) => (false, None),
            };
            Row {
                mapping,
                live,
                runtime,
            }
        })
        .collect()
}

/// The one-line status shown in the table.
pub fn status_text(row: &Row) -> &'static str {
    if row.live {
        "up"
    } else {
        "down"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_text_reflects_liveness() {
        let row = |live: bool| Row {
            mapping: Mapping {
                pane_id: "p".to_string(),
                microvm_id: "kata-p-1".to_string(),
                agent: "codex".to_string(),
                profile: "prof".to_string(),
                worktree: "/w".to_string(),
                created_at_secs: 0,
            },
            live,
            runtime: None,
        };
        assert_eq!(status_text(&row(true)), "up");
        assert_eq!(status_text(&row(false)), "down");
    }

    #[test]
    fn missing_state_means_no_rows() {
        let backend = CtrBackend {
            bin: "/definitely/not/ctr".to_string(),
            socket: "/nonexistent.sock".to_string(),
            namespace: "herdr-kata".to_string(),
            runtime: "io.containerd.kata.v2".to_string(),
        };
        let rows = load_rows(Path::new("/definitely/not/a/state/dir"), &backend);
        assert!(rows.is_empty());
    }
}

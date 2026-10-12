//! Herdr event hooks: `worktree.removed` stops the microVMs whose worktree
//! was removed and forgets their mappings.

use std::path::PathBuf;

use crate::config::load_config;
use crate::context::Env;
use crate::kata::CtrBackend;
use crate::state;

/// Extracts a worktree path from the event JSON, trying the known shapes.
fn event_worktree(env: &Env) -> Option<String> {
    let raw = env.event_json()?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    for path in [
        &["worktree", "checkout_path"] as &[&str],
        &["worktree", "path"],
        &["path"],
    ] {
        let mut current = &value;
        let mut ok = true;
        for key in path {
            match current.get(key) {
                Some(next) => current = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            if let Some(text) = current.as_str().filter(|text| !text.is_empty()) {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// Handles the event named by `HERDR_PLUGIN_EVENT`; returns the exit code.
pub fn handle_event(env: &Env) -> i32 {
    match env.event().unwrap_or("") {
        "" => {
            eprintln!("herdr-kata: HERDR_PLUGIN_EVENT is not set");
            1
        }
        "worktree.removed" => {
            let plugin_root = match env.plugin_root() {
                Some(root) => PathBuf::from(root),
                None => {
                    eprintln!("herdr-kata: HERDR_PLUGIN_ROOT is not set");
                    return 1;
                }
            };
            let config_dir = env
                .config_dir()
                .map(PathBuf::from)
                .unwrap_or_else(|| plugin_root.clone());
            let state_dir = match env.state_dir() {
                Some(dir) => PathBuf::from(dir),
                None => {
                    eprintln!("herdr-kata: HERDR_PLUGIN_STATE_DIR is not set");
                    return 1;
                }
            };
            let config = match load_config(&config_dir) {
                Ok(config) => config,
                Err(error) => {
                    eprintln!("herdr-kata: error: {}", error.message);
                    return 1;
                }
            };
            let backend = match CtrBackend::new(&config) {
                Ok(backend) => backend,
                Err(error) => {
                    eprintln!("herdr-kata: error: {}", error.message);
                    return 1;
                }
            };
            let worktree = event_worktree(env);
            let mut state = match state::load(&state_dir) {
                Ok(state) => state,
                Err(error) => {
                    eprintln!("herdr-kata: error: {}", error.message);
                    return 1;
                }
            };
            let mut stopped = 0;
            for mapping in state.mappings.clone() {
                let matches = worktree
                    .as_deref()
                    .map(|worktree| mapping.worktree == worktree)
                    .unwrap_or(false);
                if !matches {
                    continue;
                }
                backend.destroy(&mapping.microvm_id);
                state.remove_pane(&mapping.pane_id);
                stopped += 1;
                println!("stopped {} (worktree removed)", mapping.microvm_id);
            }
            if let Err(error) = state::save(&state_dir, &state) {
                eprintln!("herdr-kata: error: {}", error.message);
                return 1;
            }
            println!("herdr-kata: worktree.removed handled, {stopped} microVM(s) stopped");
            0
        }
        other => {
            eprintln!("herdr-kata: ignoring unknown event \"{other}\"");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Env;
    use std::collections::HashMap;

    fn env_with_event_json(json: &str) -> Env {
        let mut vars = HashMap::new();
        vars.insert("HERDR_PLUGIN_EVENT_JSON".to_string(), json.to_string());
        Env { vars }
    }

    #[test]
    fn worktree_extraction_tries_known_shapes() {
        let env = env_with_event_json(r#"{"worktree": {"checkout_path": "/wt"}}"#);
        assert_eq!(event_worktree(&env).as_deref(), Some("/wt"));
        let env = env_with_event_json(r#"{"path": "/other"}"#);
        assert_eq!(event_worktree(&env).as_deref(), Some("/other"));
        let env = env_with_event_json(r#"{"nothing": true}"#);
        assert_eq!(event_worktree(&env), None);
    }
}

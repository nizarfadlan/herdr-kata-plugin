//! Agent adapters: which command runs inside the Kata microVM, which profile
//! confines it, and which arguments resume a previous session.
//!
//! Unlike the nono-based plugin this mirrors, there is no client/server split:
//! one microVM per agent pane holds the whole agent.

use serde::Serialize;

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// One agent adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Adapter {
    pub title: String,
    /// Command run as the microVM's init process.
    pub command: Vec<String>,
    pub default_args: Vec<String>,
    /// Arguments that resume a previous session in a fresh microVM.
    /// Empty means the agent has no verified resume mode yet.
    pub resume_args: Vec<String>,
    /// Profile file (relative to the plugin root) describing the microVM.
    pub profile: String,
    /// Herdr's agent detection kind, when known.
    pub herdr_detection_kind: Option<String>,
}

/// The default agent adapter id.
pub fn default_agent() -> &'static str {
    "codex"
}

/// Built-in adapters keyed by agent id.
pub fn builtin_agents() -> Vec<(String, Adapter)> {
    vec![
        (
            "codex".to_string(),
            Adapter {
                title: "Codex".to_string(),
                command: words(&["codex"]),
                default_args: Vec::new(),
                // TODO: verify the real resume semantics of `codex`
                // before filling this in; do not guess.
                resume_args: Vec::new(),
                herdr_detection_kind: Some("codex".to_string()),
                profile: "profiles/herdr-codex-kata.json".to_string(),
            },
        ),
        (
            "opencode".to_string(),
            Adapter {
                title: "OpenCode".to_string(),
                command: words(&["opencode"]),
                default_args: Vec::new(),
                resume_args: words(&["--continue"]),
                herdr_detection_kind: Some("opencode".to_string()),
                profile: "profiles/herdr-opencode-kata.json".to_string(),
            },
        ),
        (
            "claude".to_string(),
            Adapter {
                title: "Claude Code".to_string(),
                command: words(&["claude"]),
                default_args: Vec::new(),
                // `claude --continue` resumes the most recent conversation.
                resume_args: words(&["--continue"]),
                herdr_detection_kind: None,
                profile: "profiles/herdr-claude-kata.json".to_string(),
            },
        ),
        (
            "pi".to_string(),
            Adapter {
                title: "Pi".to_string(),
                command: words(&["pi"]),
                default_args: Vec::new(),
                // TODO: verify pi's resume semantics before filling this in;
                // do not guess.
                resume_args: Vec::new(),
                herdr_detection_kind: None,
                profile: "profiles/herdr-pi-kata.json".to_string(),
            },
        ),
        (
            "omp".to_string(),
            Adapter {
                title: "Oh My Pi".to_string(),
                command: words(&["omp"]),
                default_args: Vec::new(),
                // `omp --resume` continues the previous session.
                resume_args: words(&["--resume"]),
                herdr_detection_kind: None,
                profile: "profiles/herdr-omp-kata.json".to_string(),
            },
        ),
    ]
}

/// Finds a built-in adapter by id.
pub fn find_adapter(id: &str) -> Option<Adapter> {
    builtin_agents()
        .into_iter()
        .find(|(name, _)| name == id)
        .map(|(_, adapter)| adapter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_is_the_default_adapter() {
        let adapter = find_adapter(default_agent()).expect("codex adapter exists");
        assert_eq!(adapter.title, "Codex");
        assert_eq!(adapter.command, words(&["codex"]));
    }

    #[test]
    fn unknown_agent_is_none() {
        assert!(find_adapter("does-not-exist").is_none());
    }

    #[test]
    fn new_adapters_resolve() {
        let claude = find_adapter("claude").expect("claude adapter exists");
        assert_eq!(claude.command, words(&["claude"]));
        assert_eq!(claude.resume_args, words(&["--continue"]));

        let pi = find_adapter("pi").expect("pi adapter exists");
        assert_eq!(pi.command, words(&["pi"]));

        let omp = find_adapter("omp").expect("omp adapter exists");
        assert_eq!(omp.command, words(&["omp"]));
        assert_eq!(omp.resume_args, words(&["--resume"]));
    }
}

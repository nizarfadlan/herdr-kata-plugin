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

/// Default guest image for an agent when neither `config.toml` nor the
/// profile sets one: the project's published image on GHCR, pinned to this
/// plugin's version so the guest entrypoint and firewall scripts stay in sync
/// with the plugin driving them.
pub fn default_guest_image(agent_id: &str) -> String {
    format!(
        "ghcr.io/nizarfadlan/herdr-kata-{agent_id}:{}",
        env!("CARGO_PKG_VERSION")
    )
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
                // `codex resume --last` reopens the most recent session for
                // the working directory (/work inside the microVM).
                resume_args: words(&["resume", "--last"]),
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
                // `pi --continue` reopens the most recent session for the
                // working directory.
                resume_args: words(&["--continue"]),
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
        assert_eq!(adapter.resume_args, words(&["resume", "--last"]));
    }

    #[test]
    fn unknown_agent_is_none() {
        assert!(find_adapter("does-not-exist").is_none());
    }

    #[test]
    fn default_guest_image_is_version_pinned() {
        let image = default_guest_image("codex");
        assert_eq!(
            image,
            format!(
                "ghcr.io/nizarfadlan/herdr-kata-codex:{}",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn new_adapters_resolve() {
        let claude = find_adapter("claude").expect("claude adapter exists");
        assert_eq!(claude.command, words(&["claude"]));
        assert_eq!(claude.resume_args, words(&["--continue"]));

        let pi = find_adapter("pi").expect("pi adapter exists");
        assert_eq!(pi.command, words(&["pi"]));
        assert_eq!(pi.resume_args, words(&["--continue"]));

        let omp = find_adapter("omp").expect("omp adapter exists");
        assert_eq!(omp.command, words(&["omp"]));
        assert_eq!(omp.resume_args, words(&["--resume"]));
    }
}

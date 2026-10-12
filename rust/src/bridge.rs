//! Runs inside a Herdr pane. Modes: `start` (first launch of the agent),
//! `connect` (launch it again with the adapter's resume arguments) and `shell`
//! (a shell in a microVM with the agent's policy).
//!
//! The bridge execs `ctr run` with the pane's stdio inherited, so the agent's
//! TUI renders in the pane. On exit it best-effort destroys the microVM
//! (`--rm` normally removes it) and drops the pane's mapping.

use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;

use crate::config::load_config;
use crate::errors::{ErrorKind, PluginError, Result};
use crate::kata::{CtrBackend, RunSpec};
use crate::lifecycle::{kata_annotations, resolve_mounts};
use crate::state;

const MODES: [&str; 3] = ["start", "connect", "shell"];

/// Environment variable carrying the egress allowlist into the guest; the
/// guest entrypoint applies the firewall from it.
pub const EGRESS_ENV: &str = "HERDR_EGRESS_ALLOW";

/// The parsed bridge argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeArgs {
    pub mode: String,
    pub state_dir: PathBuf,
    pub config_dir: PathBuf,
    pub pane_id: String,
    pub plugin_root: PathBuf,
    pub profile: PathBuf,
    pub agent_config_dir: String,
    pub microvm_id: String,
    pub image: String,
    pub worktree: PathBuf,
    pub workdir: String,
    pub agent_cmd: Vec<String>,
}

fn startup(message: impl Into<String>) -> PluginError {
    PluginError::new(ErrorKind::Startup, message)
}

/// Parses the bridge argv: flags, then `--`, then the agent command.
pub fn parse_bridge_args(argv: &[String]) -> Result<BridgeArgs> {
    let mode = argv.first().map(String::as_str).unwrap_or("undefined");
    if !MODES.contains(&mode) {
        return Err(startup(format!(
            "unknown bridge mode \"{mode}\"; expected one of {}",
            MODES.join(", ")
        )));
    }
    let dashdash = argv.iter().position(|arg| arg == "--");
    let agent_cmd: Vec<String> = match dashdash {
        Some(index) => argv[index + 1..].to_vec(),
        None => Vec::new(),
    };
    if agent_cmd.is_empty() {
        return Err(startup("missing agent command after `--`"));
    }
    let rest = &argv[1..dashdash.unwrap_or(argv.len())];
    let (mut state_dir, mut config_dir, mut pane_id, mut plugin_root) = (None, None, None, None);
    let (mut profile, mut agent_config_dir) = (None, None);
    let (mut microvm_id, mut image, mut worktree, mut workdir) = (None, None, None, None);
    for pair in rest.chunks(2) {
        let (flag, value) = (pair[0].as_str(), pair.get(1));
        let Some(value) = value else {
            return Err(startup(format!("unexpected bridge argument \"{flag}\"")));
        };
        match flag {
            "--state-dir" => state_dir = Some(PathBuf::from(value)),
            "--config-dir" => config_dir = Some(PathBuf::from(value)),
            "--pane-id" => pane_id = Some(value.clone()),
            "--plugin-root" => plugin_root = Some(PathBuf::from(value)),
            "--profile" => profile = Some(PathBuf::from(value)),
            "--agent-config-dir" => agent_config_dir = Some(value.clone()),
            "--microvm-id" => microvm_id = Some(value.clone()),
            "--image" => image = Some(value.clone()),
            "--worktree" => worktree = Some(PathBuf::from(value)),
            "--workdir" => workdir = Some(value.clone()),
            other => return Err(startup(format!("unexpected bridge argument \"{other}\""))),
        }
    }
    let require = |value: Option<PathBuf>, name: &str| {
        value.ok_or_else(|| startup(format!("missing bridge option for {name}")))
    };
    let require_str = |value: Option<String>, name: &str| {
        value
            .filter(|text| !text.is_empty())
            .ok_or_else(|| startup(format!("missing bridge option for {name}")))
    };
    Ok(BridgeArgs {
        mode: mode.to_string(),
        state_dir: require(state_dir, "stateDir")?,
        config_dir: require(config_dir, "configDir")?,
        pane_id: require_str(pane_id, "paneId")?,
        plugin_root: require(plugin_root, "pluginRoot")?,
        profile: require(profile, "profile")?,
        agent_config_dir: agent_config_dir.unwrap_or_default(),
        microvm_id: require_str(microvm_id, "microvmId")?,
        image: require_str(image, "image")?,
        worktree: require(worktree, "worktree")?,
        workdir: require_str(workdir, "workdir")?,
        agent_cmd,
    })
}

fn print_line(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// Loads the profile straight from its path (the bridge gets the resolved
/// path, not the adapter id).
fn load_profile_file(path: &PathBuf) -> Result<crate::lifecycle::VmProfile> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        PluginError::new(
            ErrorKind::Config,
            format!("cannot read profile {}: {error}", path.display()),
        )
    })?;
    serde_json::from_str(&text).map_err(|error| {
        PluginError::new(
            ErrorKind::Config,
            format!("cannot parse profile {}: {error}", path.display()),
        )
    })
}

fn run(argv: &[String]) -> Result<i32> {
    let args = parse_bridge_args(argv)?;
    let config = load_config(&args.config_dir)?;
    let backend = CtrBackend::new(&config)?;
    let profile = load_profile_file(&args.profile)?;
    let mounts = resolve_mounts(&profile, &args.worktree, &args.agent_config_dir)?;
    let annotations = kata_annotations(&profile);

    // Egress: a non-empty allowlist is enforced by the guest firewall, which
    // needs NET_ADMIN to program iptables and reads the list from the env.
    let allowlist: Vec<&str> = profile
        .egress_allowlist
        .iter()
        .map(String::as_str)
        .filter(|entry| !entry.trim().is_empty())
        .collect();
    let mut spec = RunSpec {
        id: args.microvm_id.clone(),
        image: args.image.clone(),
        workdir: args.workdir.clone(),
        mounts,
        annotations,
        command: args.agent_cmd.clone(),
        ..RunSpec::default()
    };
    if !allowlist.is_empty() {
        spec.env.push((EGRESS_ENV.to_string(), allowlist.join(",")));
        spec.cap_adds.push("NET_ADMIN".to_string());
    }

    let run_argv = backend.run_argv(&spec);
    print_line(&format!(
        "herdr-kata: starting {} in Kata microVM {} (worktree {}, {} mount(s), {} vCPU(s), {} MiB)",
        args.agent_cmd.first().map(String::as_str).unwrap_or("?"),
        args.microvm_id,
        args.worktree.display(),
        spec.mounts.len(),
        profile.vcpus,
        profile.memory_mb,
    ));
    if allowlist.is_empty() {
        print_line("herdr-kata: no egress allowlist: the guest firewall stays off");
    }
    let status = std::process::Command::new(&backend.bin)
        .args(&run_argv)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| startup(format!("cannot run {}: {error}", backend.bin)))?;
    let code = status.code().unwrap_or(1);
    print_line(&format!(
        "herdr-kata: agent exited with code {code}; removing the microVM"
    ));
    // Best-effort cleanup (`--rm` normally removes the container already)
    // and mapping removal.
    backend.destroy(&args.microvm_id);
    if let Ok(mut state) = state::load(&args.state_dir) {
        if state.remove_pane(&args.pane_id) {
            let _ = state::save(&args.state_dir, &state);
        }
    }
    Ok(code)
}

/// Entry point; returns the process exit code.
pub fn run_bridge(argv: &[String]) -> i32 {
    match run(argv) {
        Ok(code) => code,
        Err(error) => {
            print_line(&format!("herdr-kata: error: {}", error.message));
            if !error.output.trim().is_empty() {
                print_line(error.output.trim());
            }
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn parses_a_full_bridge_command() {
        let args = parse_bridge_args(&words(&[
            "start",
            "--state-dir",
            "s",
            "--config-dir",
            "c",
            "--pane-id",
            "p",
            "--plugin-root",
            "r",
            "--profile",
            "profiles/herdr-codex-kata.json",
            "--agent-config-dir",
            "",
            "--microvm-id",
            "kata-p1-abc123",
            "--image",
            "img:latest",
            "--worktree",
            "/work",
            "--workdir",
            "/work",
            "--",
            "codex",
        ]))
        .expect("parses");
        assert_eq!(args.mode, "start");
        assert_eq!(args.microvm_id, "kata-p1-abc123");
        assert_eq!(args.agent_cmd, words(&["codex"]));
        assert_eq!(
            args.profile,
            PathBuf::from("profiles/herdr-codex-kata.json")
        );
    }

    #[test]
    fn rejects_bad_modes_and_missing_parts() {
        assert!(parse_bridge_args(&words(&["attach", "--", "codex"])).is_err());
        assert!(parse_bridge_args(&words(&["start", "--", "codex"])).is_err());
        assert!(parse_bridge_args(&words(&["start", "--state-dir", "s"])).is_err());
    }
}

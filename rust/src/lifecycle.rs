//! Agent launch lifecycle: resolve the launch spec, start it in a pane,
//! stop it, verify it.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::agents::find_adapter;
use crate::config::PluginConfig;
use crate::context::{context_object, resolve_workspace_root, Env};
use crate::errors::{ErrorKind, PluginError, Result};
use crate::herdr::HerdrClient;
use crate::kata::{CtrBackend, Mount};
use crate::naming::microvm_id;
use crate::state::{self, Mapping};
use crate::util::shell_quote;
use crate::verify::{self, Check};

/// One bind mount from a VM profile.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileMount {
    pub source: String,
    pub target: String,
    /// "rw" (default) or "ro".
    #[serde(default)]
    pub mode: String,
}

/// A VM profile (`profiles/*.json`).
#[derive(Debug, Clone, Deserialize)]
pub struct VmProfile {
    pub name: String,
    #[serde(default = "default_vcpus")]
    pub vcpus: u32,
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u32,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub mounts: Vec<ProfileMount>,
    /// Host paths that must never appear among the container's mounts.
    #[serde(default)]
    pub never_mount: Vec<String>,
    /// Egress allowlist (`host:port` entries). Non-empty: the guest firewall
    /// enforces default-DROP. Empty: the firewall stays off.
    #[serde(default)]
    pub egress_allowlist: Vec<String>,
}

fn default_vcpus() -> u32 {
    2
}
fn default_memory_mb() -> u32 {
    4096
}

/// Expands a leading `~/` against `$HOME`.
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}

/// Kata OCI annotations carrying the profile's resource limits.
/// Names from the Kata Containers documentation
/// (`docs/how-to/how-to-set-sandbox-config-kata.md`); values are passed
/// through `ctr run --annotation`.
pub fn kata_annotations(profile: &VmProfile) -> Vec<(String, String)> {
    let mut annotations = Vec::new();
    if profile.vcpus > 0 {
        annotations.push((
            "io.katacontainers.config.hypervisor.default_vcpus".to_string(),
            profile.vcpus.to_string(),
        ));
    }
    if profile.memory_mb > 0 {
        annotations.push((
            "io.katacontainers.config.hypervisor.default_memory".to_string(),
            profile.memory_mb.to_string(),
        ));
    }
    annotations
}

/// Resolves a profile's mount templates against this launch:
/// `{worktree}` becomes the worktree, `{agent_config}` becomes the configured
/// agent config dir (the mount is skipped when it is not configured).
/// Anything else in braces is rejected: a typo must not silently mount the
/// wrong host path.
pub fn resolve_mounts(
    profile: &VmProfile,
    worktree: &Path,
    agent_config_dir: &str,
) -> Result<Vec<Mount>> {
    let mut mounts = Vec::new();
    for mount in &profile.mounts {
        let source = match mount.source.as_str() {
            "{worktree}" => worktree.to_string_lossy().into_owned(),
            "{agent_config}" => {
                if agent_config_dir.trim().is_empty() {
                    continue;
                }
                expand_tilde(agent_config_dir.trim())
            }
            other => {
                if other.contains('{') || other.contains('}') {
                    return Err(PluginError::new(
                        ErrorKind::Config,
                        format!(
                            "profile \"{}\": unknown mount template \"{other}\"; \
                             expected {{worktree}} or {{agent_config}}",
                            profile.name
                        ),
                    ));
                }
                other.to_string()
            }
        };
        if source.trim().is_empty() {
            return Err(PluginError::new(
                ErrorKind::Config,
                format!(
                    "profile \"{}\": mount target {} has an empty source",
                    profile.name, mount.target
                ),
            ));
        }
        mounts.push(Mount {
            source,
            target: mount.target.clone(),
            readonly: mount.mode.eq_ignore_ascii_case("ro"),
        });
    }
    Ok(mounts)
}

/// Loads and parses a profile relative to the plugin root.
pub fn load_profile(plugin_root: &Path, profile_rel: &str) -> Result<VmProfile> {
    let path = plugin_root.join(profile_rel);
    let text = std::fs::read_to_string(&path).map_err(|error| {
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

/// Everything the bridge needs to launch one agent microVM.
pub struct LaunchSpec {
    pub mode: String,
    pub pane_id: String,
    pub microvm_id: String,
    pub agent: String,
    pub image: String,
    pub worktree: PathBuf,
    pub workdir: String,
    pub agent_cmd: Vec<String>,
    pub profile: VmProfile,
    /// Absolute path of the profile file, so the bridge can re-resolve mounts.
    pub profile_path: PathBuf,
    /// The agent config dir template value (`""` when unconfigured).
    pub agent_config_dir: String,
}

/// The lifecycle driver.
pub struct Lifecycle {
    pub backend: CtrBackend,
    pub config: PluginConfig,
    pub state_dir: PathBuf,
    pub config_dir: PathBuf,
    pub plugin_root: PathBuf,
}

impl Lifecycle {
    /// Resolves the adapter, profile, image, worktree and agent command for
    /// one launch. Fails fast with a clear message when anything is missing.
    pub fn prepare(&self, env: &Env, mode: &str) -> Result<LaunchSpec> {
        let adapter = find_adapter(&self.config.agent).ok_or_else(|| {
            PluginError::new(
                ErrorKind::Config,
                format!(
                    "unknown agent \"{}\": configure agent = codex|opencode",
                    self.config.agent
                ),
            )
        })?;
        let profile = load_profile(&self.plugin_root, &adapter.profile)?;
        let image = if !self.config.image.is_empty() {
            self.config.image.clone()
        } else if !profile.image.is_empty() {
            profile.image.clone()
        } else {
            return Err(PluginError::new(
                ErrorKind::Config,
                "no guest image configured: set `image` in config.toml or in the profile, \
                 or build one from guest/Dockerfile.<agent>",
            ));
        };
        if !self.backend.image_exists(&image) {
            return Err(PluginError::new(
                ErrorKind::NotFound,
                format!(
                    "guest image {image} is not present locally: pull or build it first \
                     (see guest/Dockerfile.{})",
                    self.config.agent
                ),
            ));
        }
        let context = context_object(env).unwrap_or_default();
        let worktree = resolve_workspace_root(&context).ok_or_else(|| {
            PluginError::new(
                ErrorKind::Startup,
                "no workspace found for this pane: open Herdr in a project first",
            )
        })?;
        let microvm_id = microvm_id(env.pane_id().unwrap_or("pane"));
        let mut agent_cmd = match mode {
            "connect" => {
                let mut cmd = adapter.command.clone();
                cmd.extend(adapter.resume_args.clone());
                cmd
            }
            "shell" => vec!["sh".to_string()],
            _ => {
                let mut cmd = adapter.command.clone();
                cmd.extend(adapter.default_args.clone());
                cmd
            }
        };
        agent_cmd.extend(self.config.agent_args.clone());
        // Resolve the mounts now so a bad template fails the action before
        // any pane is split.
        let profile_path = self.plugin_root.join(&adapter.profile);
        let _ = resolve_mounts(&profile, &worktree, &self.config.agent_config_dir)?;
        Ok(LaunchSpec {
            mode: mode.to_string(),
            pane_id: String::new(),
            microvm_id,
            agent: self.config.agent.clone(),
            image,
            worktree,
            workdir: "/work".to_string(),
            agent_cmd,
            profile,
            profile_path,
            agent_config_dir: self.config.agent_config_dir.clone(),
        })
    }

    /// The `bin/run.sh bridge …` command submitted to a pane.
    fn bridge_command(&self, spec: &LaunchSpec) -> String {
        let run_sh = self.plugin_root.join("bin/run.sh");
        let mut parts = vec![
            "sh".to_string(),
            run_sh.to_string_lossy().into_owned(),
            "bridge".to_string(),
            spec.mode.clone(),
            "--state-dir".to_string(),
            self.state_dir.to_string_lossy().into_owned(),
            "--config-dir".to_string(),
            self.config_dir.to_string_lossy().into_owned(),
            "--pane-id".to_string(),
            spec.pane_id.clone(),
            "--plugin-root".to_string(),
            self.plugin_root.to_string_lossy().into_owned(),
            "--profile".to_string(),
            spec.profile_path.to_string_lossy().into_owned(),
            "--agent-config-dir".to_string(),
            spec.agent_config_dir.clone(),
            "--microvm-id".to_string(),
            spec.microvm_id.clone(),
            "--image".to_string(),
            spec.image.clone(),
            "--worktree".to_string(),
            spec.worktree.to_string_lossy().into_owned(),
            "--workdir".to_string(),
            spec.workdir.clone(),
            "--".to_string(),
        ];
        parts.extend(spec.agent_cmd.clone());
        parts
            .iter()
            .map(|part| shell_quote(part))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn record_mapping(&self, spec: &LaunchSpec) {
        let mut state = state::load(&self.state_dir).unwrap_or_default();
        state.upsert(Mapping {
            pane_id: spec.pane_id.clone(),
            microvm_id: spec.microvm_id.clone(),
            agent: spec.agent.clone(),
            profile: spec.profile.name.clone(),
            worktree: spec.worktree.to_string_lossy().into_owned(),
            created_at_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        });
        let _ = state::save(&self.state_dir, &state);
    }

    /// Splits a new pane, submits the bridge command, records the mapping.
    pub fn launch_in_new_pane(&self, herdr: &HerdrClient, spec: &mut LaunchSpec) -> Result<()> {
        let new_pane = herdr.split_pane(None, "right", &spec.worktree.to_string_lossy())?;
        spec.pane_id = new_pane.clone();
        herdr.run_in_pane(&new_pane, &self.bridge_command(spec))?;
        self.record_mapping(spec);
        Ok(())
    }

    /// Submits the bridge command in an existing pane (reconnect).
    pub fn launch_in_pane(&self, herdr: &HerdrClient, spec: &LaunchSpec) -> Result<()> {
        herdr.run_in_pane(&spec.pane_id, &self.bridge_command(spec))?;
        self.record_mapping(spec);
        Ok(())
    }

    /// Waits for the container to appear, then verifies it.
    pub fn wait_and_verify(&self, spec: &LaunchSpec) -> Vec<Check> {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(30) {
            if self.backend.container_exists(&spec.microvm_id) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        verify::verify_microvm(
            &self.backend,
            &spec.microvm_id,
            &self.backend.runtime,
            &kata_annotations(&spec.profile),
            &spec
                .profile
                .never_mount
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )
    }

    /// Stops the microVM mapped to a pane; `true` when one was tracked.
    pub fn stop_pane_agent(&self, pane_id: &str) -> Result<bool> {
        let mut state = state::load(&self.state_dir)?;
        let Some(mapping) = state.by_pane(pane_id).cloned() else {
            return Ok(false);
        };
        self.backend.destroy(&mapping.microvm_id);
        state.remove_pane(pane_id);
        state::save(&self.state_dir, &state)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_parses_with_defaults() {
        let profile: VmProfile = serde_json::from_str(
            r#"{"name": "test", "image": "img:latest", "never_mount": ["~/.ssh"]}"#,
        )
        .expect("parses");
        assert_eq!(profile.name, "test");
        assert_eq!(profile.vcpus, 2);
        assert_eq!(profile.memory_mb, 4096);
        assert_eq!(profile.never_mount, vec!["~/.ssh".to_string()]);
    }

    fn profile_with_mounts() -> VmProfile {
        VmProfile {
            name: "test".to_string(),
            vcpus: 2,
            memory_mb: 1024,
            image: String::new(),
            mounts: vec![
                ProfileMount {
                    source: "{worktree}".to_string(),
                    target: "/work".to_string(),
                    mode: "rw".to_string(),
                },
                ProfileMount {
                    source: "{agent_config}".to_string(),
                    target: "/home/agent/.codex".to_string(),
                    mode: "ro".to_string(),
                },
            ],
            never_mount: Vec::new(),
            egress_allowlist: Vec::new(),
        }
    }

    #[test]
    fn mount_templates_resolve_and_skip() {
        let profile = profile_with_mounts();
        let mounts = resolve_mounts(&profile, Path::new("/wt"), "~/.codex").expect("resolves");
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].source, "/wt");
        assert!(!mounts[0].readonly);
        assert!(mounts[1].source.ends_with(".codex"));
        assert!(mounts[1].readonly);

        let mounts = resolve_mounts(&profile, Path::new("/wt"), "").expect("resolves");
        assert_eq!(mounts.len(), 1, "the agent config mount is skipped");
    }

    #[test]
    fn unknown_templates_are_rejected() {
        let mut profile = profile_with_mounts();
        profile.mounts[0].source = "{homedir}".to_string();
        assert!(resolve_mounts(&profile, Path::new("/wt"), "").is_err());
    }

    #[test]
    fn resource_annotations_carry_the_profile_limits() {
        let annotations = kata_annotations(&profile_with_mounts());
        assert!(annotations.contains(&(
            "io.katacontainers.config.hypervisor.default_vcpus".to_string(),
            "2".to_string()
        )));
        assert!(annotations.contains(&(
            "io.katacontainers.config.hypervisor.default_memory".to_string(),
            "1024".to_string()
        )));
    }
}

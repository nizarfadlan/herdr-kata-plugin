//! Action dispatch: runs the plugin action named by `HERDR_PLUGIN_ACTION_ID`.

use std::path::PathBuf;

use crate::config::load_config;
use crate::constants::PLUGIN_ID;
use crate::context::Env;
use crate::errors::{ErrorKind, PluginError, Result};
use crate::herdr::HerdrClient;
use crate::kata::{check_prereqs, CtrBackend};
use crate::lifecycle::Lifecycle;
use crate::result::{self, ok};
use crate::state;
use crate::verify::{self, Check};

fn startup(message: impl Into<String>) -> PluginError {
    PluginError::new(ErrorKind::Startup, message)
}

/// Builds the lifecycle driver from the environment.
fn lifecycle(env: &Env) -> Result<Lifecycle> {
    let plugin_root = env
        .plugin_root()
        .map(PathBuf::from)
        .ok_or_else(|| startup("HERDR_PLUGIN_ROOT is not set"))?;
    let config_dir = env
        .config_dir()
        .map(PathBuf::from)
        .unwrap_or_else(|| plugin_root.clone());
    let state_dir = env
        .state_dir()
        .map(PathBuf::from)
        .ok_or_else(|| startup("HERDR_PLUGIN_STATE_DIR is not set"))?;
    let config = load_config(&config_dir)?;
    let backend = CtrBackend::new(&config)?;
    Ok(Lifecycle {
        backend,
        config,
        state_dir,
        config_dir,
        plugin_root,
    })
}

fn herdr_client(env: &Env) -> HerdrClient {
    HerdrClient::new(env.herdr_bin().to_string())
}

fn print_checks(checks: &[Check]) {
    for check in checks {
        let mark = if check.ok { "ok  " } else { "FAIL" };
        println!("{mark} {:<16} {}", check.name, check.detail);
    }
}

/// `doctor`: checks host prerequisites. Starts no agent.
fn doctor(env: &Env) -> Result<i32> {
    let lifecycle = lifecycle(env)?;
    println!("herdr-kata doctor (plugin {PLUGIN_ID})");
    println!("agent: {}", lifecycle.config.agent);
    let mut failed = 0;
    for check in check_prereqs(&lifecycle.config) {
        let mark = if check.ok { "ok  " } else { "FAIL" };
        println!("{mark} {:<16} {}", check.name, check.detail);
        if !check.ok {
            failed += 1;
        }
    }
    // The guest image for the configured agent.
    let (image_detail, image_ok) = match image_for(&lifecycle) {
        Ok(image) => {
            let ok = lifecycle.backend.image_exists(&image);
            let detail = if ok {
                format!("{image} is present")
            } else {
                format!(
                    "{image} is missing: pull or build it (see guest/Dockerfile.{})",
                    lifecycle.config.agent
                )
            };
            (detail, ok)
        }
        Err(error) => (error.message.clone(), false),
    };
    println!(
        "{} {:<16} {}",
        if image_ok { "ok  " } else { "FAIL" },
        "guest-image",
        image_detail,
    );
    if !image_ok {
        failed += 1;
    }
    // Hardening report: the egress policy and the agent config mount come
    // from the profile and the config; informational, not pass/fail.
    {
        use crate::agents::find_adapter;
        use crate::lifecycle::load_profile;
        if let Some(adapter) = find_adapter(&lifecycle.config.agent) {
            if let Ok(profile) = load_profile(&lifecycle.plugin_root, &adapter.profile) {
                let egress = if profile.egress_allowlist.is_empty() {
                    "no allowlist: the guest firewall stays off".to_string()
                } else {
                    format!(
                        "default-DROP, allow: {}",
                        profile.egress_allowlist.join(", ")
                    )
                };
                println!("info {:<16} {egress}", "egress");
                println!(
                    "info {:<16} {} vCPU(s), {} MiB per microVM",
                    "resources", profile.vcpus, profile.memory_mb
                );
            }
        }
        let agent_config = lifecycle.config.agent_config_dir.trim();
        println!(
            "info {:<16} {}",
            "agent-config",
            if agent_config.is_empty() {
                "not configured: the {agent_config} mount is skipped".to_string()
            } else {
                format!("{agent_config} is mounted read-only in the guest")
            }
        );
    }
    if failed > 0 {
        println!("{failed} prerequisite(s) missing; see the lines above.");
        result::print_result(
            "doctor",
            false,
            Some(ErrorKind::Kata),
            Some(&format!("{failed} prerequisite(s) missing")),
        );
        return Ok(1);
    }
    println!("all prerequisites present.");
    ok("doctor");
    Ok(0)
}

/// Resolves the guest image for the configured agent.
fn image_for(lifecycle: &Lifecycle) -> Result<String> {
    use crate::agents::find_adapter;
    use crate::lifecycle::load_profile;
    let adapter = find_adapter(&lifecycle.config.agent).ok_or_else(|| {
        PluginError::new(
            ErrorKind::Config,
            format!("unknown agent \"{}\"", lifecycle.config.agent),
        )
    })?;
    let profile = load_profile(&lifecycle.plugin_root, &adapter.profile)?;
    if !lifecycle.config.image.is_empty() {
        Ok(lifecycle.config.image.clone())
    } else if !profile.image.is_empty() {
        Ok(profile.image.clone())
    } else {
        Err(PluginError::new(
            ErrorKind::Config,
            "no guest image configured: set `image` in config.toml or in the profile",
        ))
    }
}

/// `install-keybindings`: delegates to `scripts/install-keybindings.sh`.
fn install_keybindings(env: &Env) -> Result<i32> {
    let root = env
        .plugin_root()
        .ok_or_else(|| startup("HERDR_PLUGIN_ROOT is not set"))?;
    let script = PathBuf::from(root).join("scripts/install-keybindings.sh");
    if !script.is_file() {
        return Err(startup(format!(
            "keybinding script not found at {}",
            script.display()
        )));
    }
    let status = std::process::Command::new("sh")
        .arg(&script)
        .status()
        .map_err(|error| startup(format!("cannot run install-keybindings.sh: {error}")))?;
    Ok(status.code().unwrap_or(1))
}

/// Launches an agent microVM, waits for it, verifies it.
fn launch_and_verify(env: &Env, mode: &str, in_current_pane: bool) -> Result<i32> {
    let lifecycle = lifecycle(env)?;
    let herdr = herdr_client(env);
    let mut spec = lifecycle.prepare(env, mode)?;
    if in_current_pane {
        spec.pane_id = env
            .pane_id()
            .ok_or_else(|| startup("HERDR_PANE_ID is not set; focus the agent's pane"))?
            .to_string();
        // Stop any previous microVM for this pane first.
        lifecycle.stop_pane_agent(&spec.pane_id)?;
        lifecycle.launch_in_pane(&herdr, &spec)?;
    } else {
        lifecycle.launch_in_new_pane(&herdr, &mut spec)?;
    }
    println!(
        "herdr-kata: agent starting in pane {} (microVM {})",
        spec.pane_id, spec.microvm_id
    );
    let checks = lifecycle.wait_and_verify(&spec);
    print_checks(&checks);
    if verify::all_ok(&checks) {
        ok(if in_current_pane {
            "reconnect"
        } else {
            "start-agent"
        });
        return Ok(0);
    }
    // Failed verification stops the agent, per the security design.
    let _ = lifecycle.stop_pane_agent(&spec.pane_id);
    let failed: Vec<&str> = checks
        .iter()
        .filter(|check| !check.ok)
        .map(|check| check.name)
        .collect();
    Err(PluginError::new(
        ErrorKind::Kata,
        format!(
            "microVM verification failed ({}); the agent was stopped",
            failed.join(", ")
        ),
    ))
}

/// `stop`: destroys the microVM mapped to the focused pane.
fn stop(env: &Env) -> Result<i32> {
    let lifecycle = lifecycle(env)?;
    let pane_id = env
        .pane_id()
        .ok_or_else(|| startup("HERDR_PANE_ID is not set; focus the agent's pane"))?;
    if lifecycle.stop_pane_agent(pane_id)? {
        println!("herdr-kata: stopped the microVM for pane {pane_id}");
    } else {
        println!("herdr-kata: no agent microVM is mapped to pane {pane_id}");
    }
    ok("stop");
    Ok(0)
}

/// `info`: shows the mapping plus the live container record.
fn info(env: &Env) -> Result<i32> {
    let lifecycle = lifecycle(env)?;
    let pane_id = env
        .pane_id()
        .ok_or_else(|| startup("HERDR_PANE_ID is not set; focus the agent's pane"))?;
    let state = state::load(&lifecycle.state_dir)?;
    let mapping = state.by_pane(pane_id).ok_or_else(|| {
        PluginError::new(
            ErrorKind::NotFound,
            format!("no agent microVM is mapped to pane {pane_id}"),
        )
    })?;
    println!("pane:      {}", mapping.pane_id);
    println!("microVM:   {}", mapping.microvm_id);
    println!("agent:     {}", mapping.agent);
    println!("profile:   {}", mapping.profile);
    println!("worktree:   {}", mapping.worktree);
    match lifecycle.backend.describe(&mapping.microvm_id) {
        Ok(vm) => {
            println!("runtime:   {}", vm.runtime.as_deref().unwrap_or("?"));
            println!("image:     {}", vm.image.as_deref().unwrap_or("?"));
        }
        Err(_) => println!("container: not running"),
    }
    ok("info");
    Ok(0)
}

/// `verify-microvm`: re-runs the confinement checks on the focused pane's VM.
fn verify_microvm(env: &Env) -> Result<i32> {
    let lifecycle = lifecycle(env)?;
    let pane_id = env
        .pane_id()
        .ok_or_else(|| startup("HERDR_PANE_ID is not set; focus the agent's pane"))?;
    let state = state::load(&lifecycle.state_dir)?;
    let mapping = state.by_pane(pane_id).ok_or_else(|| {
        PluginError::new(
            ErrorKind::NotFound,
            format!("no agent microVM is mapped to pane {pane_id}"),
        )
    })?;
    use crate::agents::find_adapter;
    use crate::lifecycle::{kata_annotations, load_profile};
    let adapter = find_adapter(&mapping.agent).ok_or_else(|| {
        PluginError::new(
            ErrorKind::Config,
            format!("unknown agent \"{}\"", mapping.agent),
        )
    })?;
    let profile = load_profile(&lifecycle.plugin_root, &adapter.profile)?;
    let checks = verify::verify_microvm(
        &lifecycle.backend,
        &mapping.microvm_id,
        &lifecycle.backend.runtime,
        &kata_annotations(&profile),
        &profile
            .never_mount
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    print_checks(&checks);
    if verify::all_ok(&checks) {
        ok("verify-microvm");
        Ok(0)
    } else {
        Err(PluginError::new(
            ErrorKind::Kata,
            "microVM verification failed; see the checks above",
        ))
    }
}

/// `list-microvms`: every tracked mapping and whether its container exists.
fn list_microvms(env: &Env) -> Result<i32> {
    let lifecycle = lifecycle(env)?;
    let state = state::load(&lifecycle.state_dir)?;
    if state.mappings.is_empty() {
        println!("herdr-kata: no microVMs tracked");
    }
    for mapping in &state.mappings {
        let live = lifecycle.backend.container_exists(&mapping.microvm_id);
        println!(
            "{}  {}  {}  {}",
            if live { "up  " } else { "down" },
            mapping.pane_id,
            mapping.microvm_id,
            mapping.agent,
        );
    }
    ok("list-microvms");
    Ok(0)
}

/// `prune-mappings`: forget mappings whose pane is gone (destroying the VM).
fn prune_mappings(env: &Env) -> Result<i32> {
    let lifecycle = lifecycle(env)?;
    let herdr = herdr_client(env);
    let mut state = state::load(&lifecycle.state_dir)?;
    let mut pruned = 0;
    for mapping in state.mappings.clone() {
        if herdr.pane_exists(&mapping.pane_id) {
            continue;
        }
        lifecycle.backend.destroy(&mapping.microvm_id);
        state.remove_pane(&mapping.pane_id);
        pruned += 1;
        println!("pruned {} (pane gone)", mapping.microvm_id);
    }
    state::save(&lifecycle.state_dir, &state)?;
    println!("herdr-kata: pruned {pruned} mapping(s)");
    ok("prune-mappings");
    Ok(0)
}

/// `forget-mapping`: drops the focused pane's mapping; refuses while the VM runs.
fn forget_mapping(env: &Env) -> Result<i32> {
    let lifecycle = lifecycle(env)?;
    let pane_id = env
        .pane_id()
        .ok_or_else(|| startup("HERDR_PANE_ID is not set; focus the agent's pane"))?;
    let mut state = state::load(&lifecycle.state_dir)?;
    let mapping = state.by_pane(pane_id).cloned().ok_or_else(|| {
        PluginError::new(
            ErrorKind::NotFound,
            format!("no mapping for pane {pane_id}"),
        )
    })?;
    if lifecycle.backend.container_exists(&mapping.microvm_id) {
        return Err(PluginError::new(
            ErrorKind::Kata,
            format!(
                "microVM {} is still running; stop it first",
                mapping.microvm_id
            ),
        ));
    }
    state.remove_pane(pane_id);
    state::save(&lifecycle.state_dir, &state)?;
    println!("herdr-kata: forgot the mapping for pane {pane_id}");
    ok("forget-mapping");
    Ok(0)
}

/// `microvms`: opens the interactive overlay pane.
fn microvms(env: &Env) -> Result<i32> {
    herdr_client(env).open_plugin_pane(PLUGIN_ID, "microvms", true)?;
    ok("microvms");
    Ok(0)
}

/// Entry point for the `action` subcommand; returns the process exit code.
pub fn main(env: &Env) -> i32 {
    let action = env.action_id().unwrap_or("").to_string();
    let outcome: Result<i32> = match action.as_str() {
        "" => Err(startup("HERDR_PLUGIN_ACTION_ID is not set")),
        "doctor" => doctor(env),
        "install-keybindings" => install_keybindings(env),
        "start-agent" => launch_and_verify(env, "start", false),
        "reconnect" => launch_and_verify(env, "connect", true),
        "open-shell" => launch_and_verify(env, "shell", false),
        "stop" => stop(env),
        "info" => info(env),
        "verify-microvm" => verify_microvm(env),
        "list-microvms" => list_microvms(env),
        "prune-mappings" => prune_mappings(env),
        "forget-mapping" => forget_mapping(env),
        "microvms" => microvms(env),
        other => Err(startup(format!("unknown action \"{other}\""))),
    };
    match outcome {
        Ok(code) => code,
        Err(error) => {
            if !action.is_empty() {
                result::err(&action, &error);
            }
            eprintln!("herdr-kata: error: {}", error.message);
            1
        }
    }
}

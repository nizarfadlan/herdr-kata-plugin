//! Escape probes: checks that only make sense *inside* a Kata microVM.
//!
//! The plugin launches `herdr-kata probe --json` in a throwaway microVM
//! (with the active profile); every check must pass, otherwise the profile
//! is misconfigured. These are regression tripwires: with a correct profile
//! they pass trivially, because the hostile paths are simply not there.

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;

use serde::Serialize;

/// One probe check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProbeCheck {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// The host's SSH directory must not be visible in the guest.
fn check_no_host_ssh() -> ProbeCheck {
    let candidates = ["/root/.ssh", "/home/agent/.ssh/host"];
    let present: Vec<&&str> = candidates
        .iter()
        .filter(|path| Path::new(path).exists())
        .collect();
    ProbeCheck {
        name: "no-host-ssh",
        ok: present.is_empty(),
        detail: if present.is_empty() {
            "no host SSH directory is visible".to_string()
        } else {
            format!("host SSH material visible at {}", present[0])
        },
    }
}

/// The containerd socket must not exist in the guest: with it, the agent
/// could drive the host's containerd.
fn check_no_containerd_socket() -> ProbeCheck {
    let path = "/run/containerd/containerd.sock";
    ProbeCheck {
        name: "no-containerd-socket",
        ok: !Path::new(path).exists(),
        detail: if Path::new(path).exists() {
            format!("{path} is reachable from the guest")
        } else {
            format!("{path} is not present in the guest")
        },
    }
}

/// Herdr's control socket must be unreachable. The bridge never passes
/// `HERDR_SOCKET_PATH` into the microVM; if it is set anyway, connecting
/// must fail (through it, `herdr pane run` would execute arbitrary host
/// commands).
fn check_herdr_socket_unreachable() -> ProbeCheck {
    match std::env::var("HERDR_SOCKET_PATH") {
        Err(_) => ProbeCheck {
            name: "herdr-socket-unset",
            ok: true,
            detail: "HERDR_SOCKET_PATH is not set in the guest".to_string(),
        },
        Ok(path) => match UnixStream::connect(&path) {
            Ok(_) => ProbeCheck {
                name: "herdr-socket-unset",
                ok: false,
                detail: format!("connected to the host Herdr socket at {path}"),
            },
            Err(error) => ProbeCheck {
                name: "herdr-socket-unset",
                ok: true,
                detail: format!("HERDR_SOCKET_PATH is set but unreachable ({error})"),
            },
        },
    }
}

/// Runs every probe; only meaningful inside a microVM.
pub fn run_probe() -> Vec<ProbeCheck> {
    vec![
        check_no_host_ssh(),
        check_no_containerd_socket(),
        check_herdr_socket_unreachable(),
    ]
}

/// Entry point for the hidden `probe` subcommand.
pub fn probe_main(argv: &[String]) -> ExitCode {
    let json = argv.iter().any(|arg| arg == "--json");
    let checks = run_probe();
    let ok = checks.iter().all(|check| check.ok);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&checks).expect("checks serialise")
        );
    } else {
        for check in &checks {
            let mark = if check.ok { "ok  " } else { "FAIL" };
            println!("{mark} {:<24} {}", check.name, check.detail);
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_report_a_structured_result() {
        let checks = run_probe();
        assert_eq!(checks.len(), 3);
        // No assertion on pass/fail: on the test host (not a microVM) some
        // checks legitimately fail, e.g. the containerd socket exists here.
        for check in &checks {
            assert!(!check.name.is_empty());
            assert!(!check.detail.is_empty());
        }
    }
}

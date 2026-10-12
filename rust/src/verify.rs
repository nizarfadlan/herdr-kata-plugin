//! Host-side verification: is the agent's microVM really a Kata microVM with
//! clean mounts? Every launch is verified from the host through containerd;
//! a failed verification stops the agent.

use serde::Serialize;

use crate::kata::{field, spec_mount_sources, CtrBackend};

/// One verification check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// Verifies one microVM:
/// - the container exists in the plugin's namespace,
/// - its runtime is the Kata runtime,
/// - its OCI annotations carry the profile's resource limits,
/// - none of `never_mount` host paths appear among its mount sources.
pub fn verify_microvm(
    backend: &CtrBackend,
    microvm_id: &str,
    expected_runtime: &str,
    expected_annotations: &[(String, String)],
    never_mount: &[&str],
) -> Vec<Check> {
    let mut checks = Vec::new();

    let raw = match backend.container_info_raw(microvm_id) {
        Ok(raw) => {
            checks.push(Check {
                name: "container-exists",
                ok: true,
                detail: format!("{microvm_id} exists in namespace {}", backend.namespace),
            });
            raw
        }
        Err(error) => {
            checks.push(Check {
                name: "container-exists",
                ok: false,
                detail: format!("{microvm_id} not found: {}", error.message),
            });
            return checks;
        }
    };

    let runtime = raw
        .get("Runtime")
        .and_then(|runtime| runtime.get("Name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let runtime_ok = runtime == expected_runtime;
    checks.push(Check {
        name: "kata-runtime",
        ok: runtime_ok,
        detail: if runtime_ok {
            format!("runtime is {runtime}")
        } else {
            format!("runtime is {runtime}, expected {expected_runtime}")
        },
    });

    let sources = spec_mount_sources(&raw);
    let mut violations: Vec<&str> = Vec::new();
    for forbidden in never_mount {
        // Compare expanded forms: `~/.ssh` means this user's home.
        let expanded = if let Some(rest) = forbidden.strip_prefix("~/") {
            std::env::var("HOME")
                .map(|home| format!("{home}/{rest}"))
                .unwrap_or_else(|_| forbidden.to_string())
        } else {
            forbidden.to_string()
        };
        if sources
            .iter()
            .any(|source| source == &expanded || source.starts_with(&format!("{expanded}/")))
        {
            violations.push(forbidden);
        }
    }
    checks.push(Check {
        name: "clean-mounts",
        ok: violations.is_empty(),
        detail: if violations.is_empty() {
            format!("no forbidden host paths mounted ({} mounts)", sources.len())
        } else {
            format!("forbidden host paths mounted: {}", violations.join(", "))
        },
    });

    // The resource limits the plugin asked for must be on the container's
    // OCI annotations; without them the VM would boot with Kata's defaults.
    let annotations = field(&raw, "spec")
        .and_then(|spec| field(spec, "annotations"))
        .and_then(serde_json::Value::as_object);
    let missing: Vec<&str> = expected_annotations
        .iter()
        .filter(|(key, value)| {
            annotations
                .and_then(|map| map.get(key))
                .and_then(serde_json::Value::as_str)
                != Some(value.as_str())
        })
        .map(|(key, _)| key.as_str())
        .collect();
    checks.push(Check {
        name: "resource-limits",
        ok: missing.is_empty(),
        detail: if missing.is_empty() {
            format!(
                "{} resource annotation(s) present",
                expected_annotations.len()
            )
        } else {
            format!("missing annotations: {}", missing.join(", "))
        },
    });

    checks
}

/// Whether every check passed.
pub fn all_ok(checks: &[Check]) -> bool {
    checks.iter().all(|check| check.ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info(runtime: &str, sources: Vec<&str>) -> serde_json::Value {
        json!({
            "ID": "kata-x",
            "Runtime": {"Name": runtime},
            "Spec": {"mounts": sources.iter().map(|s| json!({"source": s})).collect::<Vec<_>>()}
        })
    }

    // These tests cover the pure combinators; the containerd-backed
    // verification itself needs a live containerd (containerd smoke test).

    #[test]
    fn all_ok_requires_every_check() {
        let ok = |ok: bool| Check {
            name: "x",
            ok,
            detail: String::new(),
        };
        assert!(all_ok(&[ok(true), ok(true)]));
        assert!(!all_ok(&[ok(true), ok(false)]));
        assert!(all_ok(&[]));
    }

    #[test]
    fn info_shape_is_understood() {
        let raw = info("io.containerd.kata.v2", vec!["/work"]);
        assert_eq!(spec_mount_sources(&raw), vec!["/work".to_string()]);
    }
}

//! The Kata driver: prerequisite checks plus a `ctr`-based backend.
//!
//! The MVP backend shells out to `ctr`/`nerdctl` with direct argv (never a
//! shell string). Every flag and subcommand used here was verified against
//! containerd 2.4.1 (`ctr run/create/info/list/rm`, `tasks kill/rm`,
//! `images list -q`, and the `c info` JSON shape). A native containerd-gRPC
//! backend behind the same interface is future work.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::config::PluginConfig;
use crate::errors::{ErrorKind, PluginError, Result};
use crate::exec::{run_cli, CliOutput, ExecError, RunOptions};

/// Timeout for one driver call, so a wedged containerd cannot hang an action.
/// (The interactive `ctr run` in the bridge is not time-bounded.)
const DRIVER_TIMEOUT: Duration = Duration::from_secs(20);

/// Label marking the plugin's microVMs.
pub const MANAGED_LABEL: &str = "herdr.plugin=kata.microvm";

/// One prerequisite check result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Prereq {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

fn on_path(binary: &str) -> Option<String> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(binary))
            .find(|path| path.is_file())
            .map(|path| path.to_string_lossy().into_owned())
    })
}

/// Checks every host prerequisite for running Kata microVMs.
pub fn check_prereqs(config: &PluginConfig) -> Vec<Prereq> {
    let mut checks = Vec::new();

    let kvm = Path::new("/dev/kvm").exists();
    checks.push(Prereq {
        name: "kvm",
        ok: kvm,
        detail: if kvm {
            "/dev/kvm exists".to_string()
        } else {
            "/dev/kvm is missing: Kata needs hardware virtualization (or nested virt)".to_string()
        },
    });

    let kata_bin = on_path(&config.kata_bin);
    let kata_bin_detail = kata_bin.as_deref().map_or_else(
        || {
            format!(
                "`{}` is not on PATH: install containerd (provides ctr)",
                config.kata_bin
            )
        },
        |path| format!("{} found at {path}", config.kata_bin),
    );
    checks.push(Prereq {
        name: "containerd-cli",
        ok: kata_bin.is_some(),
        detail: kata_bin_detail,
    });

    let socket = Path::new(&config.containerd_socket);
    let socket_ok = socket.exists();
    checks.push(Prereq {
        name: "containerd-socket",
        ok: socket_ok,
        detail: if socket_ok {
            format!("{} exists", config.containerd_socket)
        } else {
            format!(
                "{} is missing: is containerd running, and does this user have socket access?",
                config.containerd_socket
            )
        },
    });

    // Heuristic: the Kata shim binary on PATH means the Kata installation is
    // present. Registration in containerd is verified by actually running a
    // microVM (on a KVM host).
    let shim = on_path("containerd-shim-kata-v2");
    checks.push(Prereq {
        name: "kata-shim",
        ok: shim.is_some(),
        detail: shim.map_or_else(
            || "containerd-shim-kata-v2 is not on PATH: install Kata Containers".to_string(),
            |path| format!("Kata shim found at {path}"),
        ),
    });

    if let (Some(bin), true) = (kata_bin.as_deref(), socket_ok) {
        let probe = run_cli(
            bin,
            &[
                "--address".to_string(),
                config.containerd_socket.clone(),
                "version".to_string(),
            ],
            &RunOptions::new(DRIVER_TIMEOUT),
        );
        let ok = matches!(&probe, Ok(output) if output.success());
        checks.push(Prereq {
            name: "containerd-api",
            ok,
            detail: match probe {
                Ok(_) if ok => "containerd answers version".to_string(),
                Ok(output) => format!("containerd version failed: {}", output.output().trim()),
                Err(ExecError::Spawn(error)) => format!("cannot start {bin}: {error}"),
                Err(ExecError::TimedOut) => "containerd version timed out".to_string(),
            },
        });
    }

    checks
}

/// A bind mount for `ctr run --mount`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub source: String,
    pub target: String,
    pub readonly: bool,
}

impl Mount {
    /// `type=bind,src=…,dst=…,options=rbind:ro|rw` (verified `ctr run --help`).
    pub fn spec(&self) -> String {
        format!(
            "type=bind,src={},dst={},options=rbind:{}",
            self.source,
            self.target,
            if self.readonly { "ro" } else { "rw" }
        )
    }
}

/// A microVM record, normalised from `ctr containers info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroVm {
    pub id: String,
    pub image: Option<String>,
    pub runtime: Option<String>,
    pub labels: Vec<(String, String)>,
}

/// Reads `.a.b.c` from a JSON value, tolerating `id`/`Id`/`ID` casings
/// (containerd uses `ID`, the OCI spec uses `mounts`).
pub(crate) fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value
        .get(key)
        .or_else(|| {
            let mut titled = String::with_capacity(key.len());
            let mut chars = key.chars();
            if let Some(first) = chars.next() {
                titled.extend(first.to_uppercase());
                titled.extend(chars);
            }
            value.get(&titled)
        })
        .or_else(|| value.get(key.to_uppercase()))
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    field(value, key)?.as_str().map(str::to_string)
}

/// Normalises one `ctr containers info` document.
pub fn normalize_info(raw: &Value) -> MicroVm {
    let labels = field(raw, "labels")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str().map(str::to_string).unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    MicroVm {
        id: string_field(raw, "id").unwrap_or_default(),
        image: string_field(raw, "image"),
        runtime: field(raw, "runtime").and_then(|runtime| string_field(runtime, "name")),
        labels,
    }
}

/// Mount sources from a `ctr containers info` document's OCI spec.
pub fn spec_mount_sources(raw: &Value) -> Vec<String> {
    field(raw, "spec")
        .and_then(|spec| field(spec, "mounts"))
        .and_then(Value::as_array)
        .map(|mounts| {
            mounts
                .iter()
                .filter_map(|mount| string_field(mount, "source"))
                .collect()
        })
        .unwrap_or_default()
}

/// Maps driver failure output to an error kind. Exit codes are not
/// documented for every `ctr` failure mode, so the output decides.
pub fn classify_failure(output: &str) -> ErrorKind {
    let text = output.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if has(&["no such file", "not found", "does not exist"]) {
        ErrorKind::NotFound
    } else if has(&[
        "permission denied",
        "operation not permitted",
        "eacces",
        "eperm",
    ]) {
        ErrorKind::Permission
    } else {
        ErrorKind::Kata
    }
}

/// The driver binary configured, or a startup error.
pub fn driver_binary(config: &PluginConfig) -> Result<String> {
    on_path(&config.kata_bin).ok_or_else(|| {
        PluginError::new(
            ErrorKind::Startup,
            format!(
                "`{}` is not on PATH; install containerd or set kata_bin",
                config.kata_bin
            ),
        )
    })
}

/// Backend driving Kata microVMs through the `ctr` CLI.
#[derive(Debug, Clone)]
pub struct CtrBackend {
    pub bin: String,
    pub socket: String,
    pub namespace: String,
    pub runtime: String,
}

impl CtrBackend {
    pub fn new(config: &PluginConfig) -> Result<Self> {
        Ok(CtrBackend {
            bin: driver_binary(config)?,
            socket: config.containerd_socket.clone(),
            namespace: config.containerd_namespace.clone(),
            runtime: config.kata_runtime.clone(),
        })
    }

    fn base_argv(&self) -> Vec<String> {
        vec![
            "--address".to_string(),
            self.socket.clone(),
            "--namespace".to_string(),
            self.namespace.clone(),
        ]
    }

    /// Runs one `ctr` subcommand; a non-zero exit becomes a classified error.
    fn run(&self, subcommand: &[&str], extra: &[String]) -> Result<CliOutput> {
        let mut argv = self.base_argv();
        argv.extend(subcommand.iter().map(|s| s.to_string()));
        argv.extend(extra.iter().cloned());
        let output = run_cli(&self.bin, &argv, &RunOptions::new(DRIVER_TIMEOUT)).map_err(
            |error| match error {
                ExecError::Spawn(error) => PluginError::new(
                    ErrorKind::Startup,
                    format!("cannot run {}: {error}", self.bin),
                ),
                ExecError::TimedOut => PluginError::new(
                    ErrorKind::Kata,
                    format!("{} {} timed out", self.bin, subcommand.join(" ")),
                ),
            },
        )?;
        if output.success() {
            return Ok(output);
        }
        let combined = output.output();
        Err(PluginError::new(
            classify_failure(&combined),
            format!(
                "{} {} failed: {}",
                self.bin,
                subcommand.join(" "),
                combined.trim()
            ),
        )
        .with_output(combined))
    }

    /// `ctr containers info <id>` → the raw JSON document.
    pub fn container_info_raw(&self, id: &str) -> Result<Value> {
        let output = self.run(&["containers", "info", id], &[])?;
        serde_json::from_str(&output.stdout).map_err(|error| {
            PluginError::new(
                ErrorKind::Kata,
                format!("cannot parse `ctr containers info` output: {error}"),
            )
            .with_output(output.stdout.clone())
        })
    }

    /// Whether the container exists.
    pub fn container_exists(&self, id: &str) -> bool {
        self.container_info_raw(id).is_ok()
    }

    /// Normalised record for one container.
    pub fn describe(&self, id: &str) -> Result<MicroVm> {
        Ok(normalize_info(&self.container_info_raw(id)?))
    }

    /// Best-effort stop: `tasks kill`, `tasks rm`, `containers rm`.
    /// Missing containers are not an error.
    pub fn destroy(&self, id: &str) {
        let _ = self.run(&["tasks", "kill", id], &[]);
        let _ = self.run(&["tasks", "rm", id], &[]);
        let _ = self.run(&["containers", "rm", id], &[]);
    }

    /// Whether an image ref is present locally (`ctr images list -q`).
    pub fn image_exists(&self, image: &str) -> bool {
        self.run(&["images", "list", "-q"], &[])
            .map(|output| {
                output
                    .stdout
                    .lines()
                    .any(|line| line.trim() == image.trim())
            })
            .unwrap_or(false)
    }

    /// Builds the full `ctr run` argv (global flags included).
    pub fn run_argv(&self, spec: &RunSpec) -> Vec<String> {
        let mut argv = self.base_argv();
        argv.push("run".to_string());
        argv.extend(
            [
                "--runtime",
                &self.runtime,
                "--rm",
                "--tty",
                "--cwd",
                &spec.workdir,
                "--label",
                MANAGED_LABEL,
            ]
            .iter()
            .map(|s| s.to_string()),
        );
        for (key, value) in &spec.annotations {
            argv.push("--annotation".to_string());
            argv.push(format!("{key}={value}"));
        }
        for (key, value) in &spec.env {
            argv.push("--env".to_string());
            argv.push(format!("{key}={value}"));
        }
        for cap in &spec.cap_adds {
            argv.push("--cap-add".to_string());
            argv.push(cap.clone());
        }
        for mount in &spec.mounts {
            argv.push("--mount".to_string());
            argv.push(mount.spec());
        }
        argv.push(spec.image.clone());
        argv.push(spec.id.clone());
        argv.extend(spec.command.iter().cloned());
        argv
    }
}

/// Everything `ctr run` needs for one microVM launch.
#[derive(Debug, Clone, Default)]
pub struct RunSpec {
    pub id: String,
    pub image: String,
    pub workdir: String,
    pub mounts: Vec<Mount>,
    /// OCI annotations, e.g. the Kata resource annotations.
    pub annotations: Vec<(String, String)>,
    /// Environment variables for the guest (`--env KEY=VALUE`).
    pub env: Vec<(String, String)>,
    /// Linux capabilities, e.g. `NET_ADMIN` for the guest egress firewall.
    pub cap_adds: Vec<String>,
    pub command: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_failure_maps_common_output() {
        assert_eq!(
            classify_failure("ctr: container \"x\" in namespace \"n\": not found"),
            ErrorKind::NotFound
        );
        assert_eq!(
            classify_failure("permission denied while connecting"),
            ErrorKind::Permission
        );
        assert_eq!(classify_failure("something else broke"), ErrorKind::Kata);
    }

    #[test]
    fn normalize_info_reads_the_verified_shape() {
        // Shape captured from `ctr containers info` (containerd 2.4.1).
        let raw = json!({
            "ID": "kata-x",
            "Image": "img:latest",
            "Runtime": {"Name": "io.containerd.kata.v2"},
            "Labels": {"herdr.plugin": "kata.microvm"},
            "Spec": {"mounts": [{"source": "/work", "destination": "/work"}]},
        });
        let vm = normalize_info(&raw);
        assert_eq!(vm.id, "kata-x");
        assert_eq!(vm.runtime.as_deref(), Some("io.containerd.kata.v2"));
        assert_eq!(
            vm.labels,
            vec![("herdr.plugin".to_string(), "kata.microvm".to_string())]
        );
        assert_eq!(spec_mount_sources(&raw), vec!["/work".to_string()]);
    }

    #[test]
    fn mount_spec_uses_the_verified_syntax() {
        let mount = Mount {
            source: "/host/wt".to_string(),
            target: "/work".to_string(),
            readonly: false,
        };
        assert_eq!(
            mount.spec(),
            "type=bind,src=/host/wt,dst=/work,options=rbind:rw"
        );
    }

    #[test]
    fn run_argv_pins_the_kata_runtime() {
        let backend = CtrBackend {
            bin: "ctr".to_string(),
            socket: "/run/containerd/containerd.sock".to_string(),
            namespace: "herdr-kata".to_string(),
            runtime: "io.containerd.kata.v2".to_string(),
        };
        let argv = backend.run_argv(&RunSpec {
            id: "kata-1".to_string(),
            image: "img".to_string(),
            workdir: "/work".to_string(),
            annotations: vec![(
                "io.katacontainers.config.hypervisor.default_memory".to_string(),
                "4096".to_string(),
            )],
            env: vec![("HERDR_EGRESS_ALLOW".to_string(), "proxy:8080".to_string())],
            cap_adds: vec!["NET_ADMIN".to_string()],
            command: vec!["codex".to_string()],
            ..RunSpec::default()
        });
        let runtime = argv
            .windows(2)
            .find(|w| w[0] == "--runtime")
            .expect("has --runtime");
        assert_eq!(runtime[1], "io.containerd.kata.v2");
        assert!(argv.contains(&"--rm".to_string()));
        assert!(argv.contains(&"--tty".to_string()));
        assert!(argv.contains(&"herdr.plugin=kata.microvm".to_string()));
        let annotation = argv
            .windows(2)
            .find(|w| w[0] == "--annotation")
            .expect("has --annotation");
        assert_eq!(
            annotation[1],
            "io.katacontainers.config.hypervisor.default_memory=4096"
        );
        let env = argv
            .windows(2)
            .find(|w| w[0] == "--env")
            .expect("has --env");
        assert_eq!(env[1], "HERDR_EGRESS_ALLOW=proxy:8080");
        let cap = argv
            .windows(2)
            .find(|w| w[0] == "--cap-add")
            .expect("has --cap-add");
        assert_eq!(cap[1], "NET_ADMIN");
        assert_eq!(argv.last().expect("non-empty"), "codex");
    }
}

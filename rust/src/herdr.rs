//! Thin client around the Herdr CLI (`HERDR_BIN_PATH`): the whole CLI is the
//! plugin API. Every call is a direct argv (never a shell string) with a
//! timeout, and JSON responses are read defensively.

use std::time::Duration;

use serde_json::Value;

use crate::errors::{ErrorKind, PluginError, Result};
use crate::exec::{run_cli, CliOutput, ExecError, RunOptions};

/// Timeout for one Herdr CLI call.
const HERDR_TIMEOUT: Duration = Duration::from_secs(15);

/// How a Herdr CLI call can fail.
#[derive(Debug)]
enum CallFailure {
    /// The binary could not be started.
    Spawn(std::io::Error),
    /// The call took too long.
    TimedOut,
    /// The command ran and reported an error (e.g. `pane get` on a missing pane).
    CommandFailed(CliOutput),
}

/// Walks a JSON value down a key path.
fn at<'a>(json: Option<&'a Value>, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(json?, |value, key| value.get(*key))
}

fn json_string(value: Option<&Value>) -> Option<String> {
    value?.as_str().map(str::to_string)
}

/// Client for the Herdr CLI.
#[derive(Debug, Clone)]
pub struct HerdrClient {
    bin: String,
}

impl HerdrClient {
    pub fn new(bin: String) -> Self {
        HerdrClient { bin }
    }

    fn call(&self, args: &[&str]) -> Result<CliOutput> {
        match self.try_call(args) {
            Ok(output) => Ok(output),
            Err(CallFailure::Spawn(error)) => Err(PluginError::new(
                ErrorKind::Startup,
                format!("cannot run `{}`: {error}", self.bin),
            )),
            Err(CallFailure::TimedOut) => Err(PluginError::new(
                ErrorKind::Unknown,
                format!("`{} {}` timed out", self.bin, args.join(" ")),
            )),
            Err(CallFailure::CommandFailed(output)) => Err(PluginError::new(
                ErrorKind::Unknown,
                format!(
                    "`{} {}` failed: {}",
                    self.bin,
                    args.join(" "),
                    output.output().trim()
                ),
            )
            .with_output(output.output())),
        }
    }

    /// Runs the CLI, distinguishing transport failures from a command that
    /// simply reported an error (e.g. `pane get` on a missing pane).
    fn try_call(&self, args: &[&str]) -> std::result::Result<CliOutput, CallFailure> {
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        match run_cli(&self.bin, &argv, &RunOptions::new(HERDR_TIMEOUT)) {
            Ok(output) if output.success() => Ok(output),
            Ok(output) => Err(CallFailure::CommandFailed(output)),
            Err(ExecError::Spawn(error)) => Err(CallFailure::Spawn(error)),
            Err(ExecError::TimedOut) => Err(CallFailure::TimedOut),
        }
    }

    fn call_json(&self, args: &[&str]) -> Result<Option<Value>> {
        // `call` already errors when the command fails.
        let output = self.call(args)?;
        Ok(serde_json::from_str(&output.stdout).ok())
    }

    /// Splits a pane; returns the new pane id.
    /// `herdr pane split [--pane ID] --direction right|down [--cwd PATH] [--focus]`
    /// exposes the id at `.result.pane.pane_id`.
    pub fn split_pane(&self, source: Option<&str>, direction: &str, cwd: &str) -> Result<String> {
        let mut args = vec!["pane", "split", "--direction", direction, "--cwd", cwd];
        if let Some(source) = source {
            args.insert(1, "--pane");
            args.insert(2, source);
        }
        let json = self.call_json(&args)?;
        json_string(at(json.as_ref(), &["result", "pane", "pane_id"]))
            .or_else(|| json_string(at(json.as_ref(), &["pane", "pane_id"])))
            .ok_or_else(|| {
                PluginError::new(
                    ErrorKind::Unknown,
                    "herdr pane split did not return a pane id",
                )
            })
    }

    /// Submits a command plus Enter in a pane (`herdr pane run`).
    pub fn run_in_pane(&self, pane_id: &str, command: &str) -> Result<()> {
        self.call(&["pane", "run", pane_id, command])?;
        Ok(())
    }

    /// The raw `herdr pane get` document, or `None` when the pane is gone.
    /// Transport failures (Herdr not running) are still errors.
    pub fn pane_info(&self, pane_id: &str) -> Result<Option<Value>> {
        match self.try_call(&["pane", "get", pane_id]) {
            Ok(output) => Ok(serde_json::from_str(&output.stdout).ok()),
            Err(CallFailure::CommandFailed(_)) => Ok(None),
            Err(CallFailure::Spawn(error)) => Err(PluginError::new(
                ErrorKind::Startup,
                format!("cannot run `{}`: {error}", self.bin),
            )),
            Err(CallFailure::TimedOut) => Err(PluginError::new(
                ErrorKind::Unknown,
                format!("`{} pane get` timed out", self.bin),
            )),
        }
    }

    /// Whether Herdr still knows a pane (`herdr pane get`).
    pub fn pane_exists(&self, pane_id: &str) -> bool {
        self.pane_info(pane_id)
            .map(|json| {
                let pane =
                    at(json.as_ref(), &["result", "pane"]).or_else(|| at(json.as_ref(), &["pane"]));
                pane.is_some()
            })
            .unwrap_or(false)
    }

    /// Ids of every pane Herdr knows (`herdr pane list`).
    pub fn pane_ids(&self) -> Result<Vec<String>> {
        let json = self.call_json(&["pane", "list"])?;
        let panes = at(json.as_ref(), &["result", "panes"])
            .or_else(|| at(json.as_ref(), &["panes"]))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                PluginError::new(
                    ErrorKind::Unknown,
                    "herdr pane list did not return a pane list",
                )
            })?;
        Ok(panes
            .iter()
            .filter_map(|pane| json_string(pane.get("pane_id")))
            .filter(|id| !id.is_empty())
            .collect())
    }

    /// Closes a pane (`herdr pane close`).
    pub fn close_pane(&self, pane_id: &str) -> Result<()> {
        self.call(&["pane", "close", pane_id])?;
        Ok(())
    }

    /// Opens a plugin pane entrypoint
    /// (`herdr plugin pane open --plugin <id> --entrypoint <id> [--focus]`).
    pub fn open_plugin_pane(&self, plugin_id: &str, entrypoint: &str, focus: bool) -> Result<()> {
        let mut args = vec![
            "plugin",
            "pane",
            "open",
            "--plugin",
            plugin_id,
            "--entrypoint",
            entrypoint,
        ];
        if focus {
            args.push("--focus");
        }
        self.call(&args)?;
        Ok(())
    }

    /// Jumps to a pane by focusing its workspace and tab
    /// (`herdr workspace focus`, `herdr tab focus`).
    pub fn focus_pane(&self, pane_id: &str) -> Result<()> {
        let json = self.pane_info(pane_id)?.ok_or_else(|| {
            PluginError::new(
                ErrorKind::NotFound,
                format!("pane {pane_id} no longer exists"),
            )
        })?;
        let pane = at(Some(&json), &["result", "pane"])
            .or_else(|| at(Some(&json), &["pane"]))
            .unwrap_or(&json);
        let workspace = json_string(pane.get("workspace_id"))
            .or_else(|| pane_id.split_once(':').map(|(ws, _)| ws.to_string()));
        if let Some(workspace) = workspace {
            self.call(&["workspace", "focus", &workspace])?;
        }
        if let Some(tab) = json_string(pane.get("tab_id")) {
            self.call(&["tab", "focus", &tab])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Writes a fake `herdr` script answering canned JSON, returns its path.
    fn fake_herdr() -> String {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("herdr");
        // Close the write handle in its own scope before the script is ever
        // executed: on some filesystems exec'ing a just-written file can hit
        // ETXTBSY ("Text file busy") while a writer is still open.
        {
            let mut file = std::fs::File::create(&path).expect("create");
            writeln!(
                file,
            r#"#!/bin/sh
case "$1 $2" in
  "pane split") echo '{{"result":{{"pane":{{"pane_id":"pane-new-1"}}}}}}' ;;
  "pane get") if [ "$3" = "gone" ]; then echo '{{"error":{{"code":"pane_not_found"}}}}' >&2; exit 1; fi
    echo '{{"result":{{"pane":{{"pane_id":"$3"}}}}}}' ;;
  "pane list") echo '{{"result":{{"panes":[{{"pane_id":"a:1"}},{{"pane_id":""}},{{"x":1}},{{"pane_id":"b:2"}}]}}}}' ;;
  "pane run") echo '{{"result":{{"ok":true}}}}' ;;
  "pane close") echo 'plain text, not JSON' ;;
  *) echo "unexpected: $*" >&2; exit 3 ;;
esac"#
        )
        .expect("write");
            file.sync_all().expect("sync");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        // Leak the tempdir so the script survives the test.
        let path_str = path.to_string_lossy().into_owned();
        std::mem::forget(dir);
        path_str
    }

    #[test]
    fn split_returns_the_new_pane_id() {
        let herdr = HerdrClient::new(fake_herdr());
        let id = herdr.split_pane(None, "right", "/work").expect("split");
        assert_eq!(id, "pane-new-1");
    }

    #[test]
    fn pane_existence_and_listing() {
        let herdr = HerdrClient::new(fake_herdr());
        assert!(herdr.pane_exists("a:1"));
        assert!(!herdr.pane_exists("gone"));
        assert_eq!(herdr.pane_ids().expect("list"), vec!["a:1", "b:2"]);
    }

    #[test]
    fn run_and_close_tolerate_plain_text() {
        let herdr = HerdrClient::new(fake_herdr());
        herdr.run_in_pane("a:1", "echo hi").expect("run");
        herdr.close_pane("a:1").expect("close");
    }
}

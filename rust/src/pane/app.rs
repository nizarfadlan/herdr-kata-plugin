//! The overlay application: key handling and the actions behind the keys.
//!
//! Actions run synchronously: containerd calls are local and fast, which
//! keeps the code (and its tests) simple. A background refresh worker is a
//! possible later improvement.

use std::path::Path;

use crate::agents::find_adapter;
use crate::herdr::HerdrClient;
use crate::kata::CtrBackend;
use crate::lifecycle::{kata_annotations, load_profile};
use crate::pane::model::{self, Row};
use crate::state;
use crate::verify;

/// The keys the overlay understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Enter,
    Esc,
    CtrlC,
}

/// What the event loop should do after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
}

/// A pending yes/no question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingConfirm {
    pub prompt: String,
    pub action: ConfirmAction,
}

/// The action a confirmed question runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    Stop { pane_id: String, microvm_id: String },
    CleanAll,
}

/// The dependencies the actions need.
pub struct Ops<'a> {
    pub backend: &'a CtrBackend,
    pub herdr: &'a HerdrClient,
    pub state_dir: &'a Path,
    pub plugin_root: &'a Path,
}

/// The overlay state.
pub struct App {
    pub rows: Vec<Row>,
    pub selected: usize,
    pub detail: bool,
    pub help: bool,
    pub status: Option<String>,
    pub confirm: Option<PendingConfirm>,
}

impl App {
    pub fn new(rows: Vec<Row>) -> App {
        App {
            rows,
            selected: 0,
            detail: false,
            help: false,
            status: None,
            confirm: None,
        }
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    fn say(&mut self, message: impl Into<String>) {
        self.status = Some(message.into());
    }

    fn clamp_selection(&mut self) {
        if self.rows.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.rows.len() - 1);
        }
    }

    fn refresh(&mut self, ops: &Ops) {
        self.rows = model::load_rows(ops.state_dir, ops.backend);
        self.clamp_selection();
    }

    fn stop_one(&mut self, ops: &Ops, pane_id: &str, microvm_id: &str) {
        ops.backend.destroy(microvm_id);
        let removed = state::load(ops.state_dir)
            .map(|mut state| {
                let removed = state.remove_pane(pane_id);
                let _ = state::save(ops.state_dir, &state);
                removed
            })
            .unwrap_or(false);
        self.refresh(ops);
        self.say(format!(
            "stopped {microvm_id}{}",
            if removed {
                ""
            } else {
                " (no mapping was left)"
            }
        ));
    }

    fn verify_selected(&mut self, ops: &Ops) {
        let Some(row) = self.selected_row() else {
            self.say("nothing to verify: no microVMs tracked");
            return;
        };
        let (pane_id, microvm_id, agent) = (
            row.mapping.pane_id.clone(),
            row.mapping.microvm_id.clone(),
            row.mapping.agent.clone(),
        );
        if !row.live {
            self.say(format!("{microvm_id} is not running; nothing to verify"));
            return;
        }
        let profile = match find_adapter(&agent)
            .ok_or_else(|| format!("unknown agent \"{agent}\""))
            .and_then(|adapter| {
                load_profile(ops.plugin_root, &adapter.profile)
                    .map_err(|error| error.message.clone())
            }) {
            Ok(profile) => profile,
            Err(message) => {
                self.say(format!("cannot verify {microvm_id}: {message}"));
                return;
            }
        };
        let checks = verify::verify_microvm(
            ops.backend,
            &microvm_id,
            &ops.backend.runtime,
            &kata_annotations(&profile),
            &profile
                .never_mount
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );
        let failed: Vec<&str> = checks
            .iter()
            .filter(|check| !check.ok)
            .map(|check| check.name)
            .collect();
        if failed.is_empty() {
            self.say(format!(
                "{microvm_id} verified: {}/{} checks ok",
                checks.len(),
                checks.len()
            ));
        } else {
            self.say(format!("{microvm_id} FAILED: {}", failed.join(", ")));
        }
        let _ = pane_id;
    }

    fn prune(&mut self, ops: &Ops) {
        let mut pruned = 0;
        for row in model::load_rows(ops.state_dir, ops.backend) {
            if ops.herdr.pane_exists(&row.mapping.pane_id) {
                continue;
            }
            ops.backend.destroy(&row.mapping.microvm_id);
            if let Ok(mut state) = state::load(ops.state_dir) {
                state.remove_pane(&row.mapping.pane_id);
                let _ = state::save(ops.state_dir, &state);
            }
            pruned += 1;
        }
        self.refresh(ops);
        self.say(if pruned == 0 {
            "nothing to prune: every pane still exists".to_string()
        } else {
            format!("pruned {pruned} mapping(s) whose pane is gone")
        });
    }

    fn clean_all(&mut self, ops: &Ops) {
        let rows = model::load_rows(ops.state_dir, ops.backend);
        let total = rows.len();
        let mut stopped = 0;
        for row in &rows {
            if row.live {
                ops.backend.destroy(&row.mapping.microvm_id);
                stopped += 1;
            }
        }
        let _ = state::save(ops.state_dir, &state::State::default());
        self.refresh(ops);
        self.say(format!(
            "stopped {stopped} running microVM(s), forgot {total} mapping(s)"
        ));
    }

    fn ask(&mut self, prompt: impl Into<String>, action: ConfirmAction) {
        self.confirm = Some(PendingConfirm {
            prompt: prompt.into(),
            action,
        });
    }

    /// Handles one key; returns whether the event loop should quit.
    pub fn on_key(&mut self, key: Key, ops: &mut Ops) -> Effect {
        if matches!(key, Key::CtrlC) {
            return Effect::Quit;
        }
        // A pending question eats every key except y/n/Esc.
        if let Some(pending) = self.confirm.take() {
            match key {
                Key::Char('y') | Key::Char('Y') => match pending.action {
                    ConfirmAction::Stop {
                        pane_id,
                        microvm_id,
                    } => self.stop_one(ops, &pane_id, &microvm_id),
                    ConfirmAction::CleanAll => self.clean_all(ops),
                },
                _ => self.say("cancelled"),
            }
            return Effect::None;
        }
        if self.help {
            if matches!(key, Key::Esc | Key::Char('?')) {
                self.help = false;
            }
            return Effect::None;
        }
        if self.detail && matches!(key, Key::Esc | Key::Enter) {
            self.detail = false;
            return Effect::None;
        }
        match key {
            Key::Char('q') => return Effect::Quit,
            Key::Up | Key::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
            }
            Key::Down | Key::Char('j') => {
                if !self.rows.is_empty() {
                    self.selected = (self.selected + 1).min(self.rows.len() - 1);
                }
            }
            Key::Char('?') => self.help = true,
            Key::Enter => {
                self.detail = self.selected_row().is_some();
            }
            Key::Esc => self.status = None,
            Key::Char('r') => {
                self.refresh(ops);
                self.say("refreshed");
            }
            Key::Char('v') => self.verify_selected(ops),
            Key::Char('x') => {
                let Some(row) = self.selected_row() else {
                    self.say("nothing to stop: no microVMs tracked");
                    return Effect::None;
                };
                let (pane_id, microvm_id) =
                    (row.mapping.pane_id.clone(), row.mapping.microvm_id.clone());
                if !row.live {
                    self.say(format!("{microvm_id} is not running; nothing to stop"));
                } else {
                    self.ask(
                        format!("Stop {microvm_id} (pane {pane_id})?"),
                        ConfirmAction::Stop {
                            pane_id,
                            microvm_id,
                        },
                    );
                }
            }
            Key::Char('p') => self.prune(ops),
            Key::Char('a') => {
                if self.rows.is_empty() {
                    self.say("nothing to clean up: no microVMs tracked");
                } else {
                    let running = self.rows.iter().filter(|row| row.live).count();
                    self.ask(
                        format!(
                            "Stop {running} running microVM(s) and forget all {} mapping(s)?",
                            self.rows.len()
                        ),
                        ConfirmAction::CleanAll,
                    );
                }
            }
            Key::Char('g') => {
                let Some(row) = self.selected_row() else {
                    return Effect::None;
                };
                let (pane_id, microvm_id) =
                    (row.mapping.pane_id.clone(), row.mapping.microvm_id.clone());
                match ops.herdr.focus_pane(&pane_id) {
                    Ok(()) => return Effect::Quit,
                    Err(error) => {
                        self.say(format!("cannot jump to {microvm_id}: {}", error.message))
                    }
                }
            }
            Key::Char(_) | Key::CtrlC => {}
        }
        Effect::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::model::Row;
    use crate::state::Mapping;

    fn row(pane_id: &str, live: bool) -> Row {
        Row {
            mapping: Mapping {
                pane_id: pane_id.to_string(),
                microvm_id: format!("kata-{pane_id}"),
                agent: "codex".to_string(),
                profile: "prof".to_string(),
                worktree: "/w".to_string(),
                created_at_secs: 0,
            },
            live,
            runtime: None,
        }
    }

    fn ops<'a>() -> (Ops<'a>, tempfile::TempDir) {
        // The backend/herdr are never called by these navigation tests.
        let dir = tempfile::tempdir().expect("tempdir");
        let ops = Ops {
            backend: Box::leak(Box::new(CtrBackend {
                bin: "/definitely/not/ctr".to_string(),
                socket: "/nonexistent.sock".to_string(),
                namespace: "herdr-kata".to_string(),
                runtime: "io.containerd.kata.v2".to_string(),
            })),
            herdr: Box::leak(Box::new(HerdrClient::new(
                "/definitely/not/herdr".to_string(),
            ))),
            state_dir: Box::leak(Box::new(dir.path().to_path_buf())),
            plugin_root: Box::leak(Box::new(dir.path().to_path_buf())),
        };
        (ops, dir)
    }

    #[test]
    fn navigation_clamps_to_the_rows() {
        let (mut ops, _dir) = ops();
        let mut app = App::new(vec![row("a", true), row("b", false)]);
        assert_eq!(app.on_key(Key::Char('q'), &mut ops), Effect::Quit);
        let mut app = App::new(vec![row("a", true), row("b", false)]);
        app.on_key(Key::Char('j'), &mut ops);
        assert_eq!(app.selected, 1);
        app.on_key(Key::Char('j'), &mut ops);
        assert_eq!(app.selected, 1, "stays at the last row");
        app.on_key(Key::Char('k'), &mut ops);
        assert_eq!(app.selected, 0);
        app.on_key(Key::Char('k'), &mut ops);
        assert_eq!(app.selected, 0, "stays at the first row");
    }

    #[test]
    fn enter_toggles_details_and_escape_closes_them() {
        let (mut ops, _dir) = ops();
        let mut app = App::new(vec![row("a", true)]);
        app.on_key(Key::Enter, &mut ops);
        assert!(app.detail);
        app.on_key(Key::Esc, &mut ops);
        assert!(!app.detail);
    }

    #[test]
    fn help_opens_and_closes() {
        let (mut ops, _dir) = ops();
        let mut app = App::new(vec![]);
        app.on_key(Key::Char('?'), &mut ops);
        assert!(app.help);
        app.on_key(Key::Char('?'), &mut ops);
        assert!(!app.help);
    }

    #[test]
    fn actions_on_an_empty_list_report_nothing() {
        let (mut ops, _dir) = ops();
        let mut app = App::new(vec![]);
        app.on_key(Key::Char('v'), &mut ops);
        assert!(app
            .status
            .as_deref()
            .unwrap_or("")
            .contains("nothing to verify"));
        app.on_key(Key::Char('x'), &mut ops);
        assert!(app
            .status
            .as_deref()
            .unwrap_or("")
            .contains("nothing to stop"));
        app.on_key(Key::Char('a'), &mut ops);
        assert!(app
            .status
            .as_deref()
            .unwrap_or("")
            .contains("nothing to clean up"));
        assert_eq!(app.on_key(Key::Char('g'), &mut ops), Effect::None);
    }

    #[test]
    fn a_stop_question_can_be_cancelled() {
        let (mut ops, _dir) = ops();
        let mut app = App::new(vec![row("a", true)]);
        app.on_key(Key::Char('x'), &mut ops);
        assert!(app.confirm.is_some());
        app.on_key(Key::Esc, &mut ops);
        assert!(app.confirm.is_none());
        assert_eq!(app.status.as_deref(), Some("cancelled"));
    }

    #[test]
    fn ctrl_c_quits_even_with_a_pending_question() {
        let (mut ops, _dir) = ops();
        let mut app = App::new(vec![row("a", true)]);
        app.on_key(Key::Char('x'), &mut ops);
        assert_eq!(app.on_key(Key::CtrlC, &mut ops), Effect::Quit);
    }
}

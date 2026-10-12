//! The `microvms` overlay pane: every tracked agent microVM, its pane and its
//! verification state. Keys: `?` help, `enter` details, `g` jump to the pane,
//! `v` verify, `x` stop, `p` prune, `a` clean up all, `r` refresh, `q` quit.
//!
//! When stdout is not a terminal (a pipe, a test), the overlay prints the
//! table as plain text instead of starting the TUI.

pub mod app;
pub mod model;
pub mod ui;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::crossterm::ExecutableCommand;
use ratatui::Terminal;

use crate::config::load_config;
use crate::context::Env;
use crate::errors::{ErrorKind, PluginError, Result};
use crate::herdr::HerdrClient;
use crate::kata::CtrBackend;
use crate::pane::app::{App, Effect, Key, Ops};
use crate::pane::model::{load_rows, status_text};
use crate::pane::ui::render;

fn startup(message: impl Into<String>) -> PluginError {
    PluginError::new(ErrorKind::Startup, message)
}

/// Builds the dependencies the overlay needs from the environment.
fn build_deps(env: &Env) -> Result<(CtrBackend, HerdrClient, PathBuf, PathBuf)> {
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
    let herdr = HerdrClient::new(env.herdr_bin().to_string());
    Ok((backend, herdr, state_dir, plugin_root))
}

/// Plain-text fallback for non-terminal output.
fn print_text(backend: &CtrBackend, state_dir: &std::path::Path) {
    let rows = load_rows(state_dir, backend);
    if rows.is_empty() {
        println!("No microVMs tracked.");
        return;
    }
    println!(
        "{:<6} {:<14} {:<26} {:<8} runtime",
        "state", "pane", "microVM", "agent"
    );
    for row in &rows {
        println!(
            "{:<6} {:<14} {:<26} {:<8} {}",
            status_text(row),
            row.mapping.pane_id,
            row.mapping.microvm_id,
            row.mapping.agent,
            row.runtime.as_deref().unwrap_or("-"),
        );
    }
}

fn to_key(code: KeyCode, modifiers: KeyModifiers) -> Option<Key> {
    if modifiers.contains(KeyModifiers::CONTROL) && matches!(code, KeyCode::Char('c')) {
        return Some(Key::CtrlC);
    }
    Some(match code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        _ => return None,
    })
}

fn restore_terminal() {
    let mut stdout = std::io::stdout();
    let _ = stdout.execute(LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

/// Runs the interactive overlay; returns the process exit code.
fn run_tui(
    backend: &CtrBackend,
    herdr: &HerdrClient,
    state_dir: &PathBuf,
    plugin_root: &PathBuf,
) -> i32 {
    if enable_raw_mode().is_err() {
        eprintln!("herdr-kata: cannot enable raw mode; is this a terminal?");
        return 1;
    }
    if std::io::stdout().execute(EnterAlternateScreen).is_err() {
        let _ = disable_raw_mode();
        eprintln!("herdr-kata: cannot enter the alternate screen");
        return 1;
    }
    let mut terminal = match Terminal::new(CrosstermBackend::new(std::io::stdout())) {
        Ok(terminal) => terminal,
        Err(error) => {
            restore_terminal();
            eprintln!("herdr-kata: cannot start the terminal: {error}");
            return 1;
        }
    };

    let mut app = App::new(load_rows(state_dir, backend));
    let mut ops = Ops {
        backend,
        herdr,
        state_dir,
        plugin_root,
    };
    loop {
        if terminal.draw(|frame| render(frame, &app)).is_err() {
            break;
        }
        let key = match event::poll(Duration::from_millis(250)) {
            Ok(true) => match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    to_key(key.code, key.modifiers)
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(key) = key {
            if app.on_key(key, &mut ops) == Effect::Quit {
                break;
            }
        }
    }
    restore_terminal();
    0
}

/// Entry point for the `pane` subcommand; returns the process exit code.
pub fn run_pane() -> i32 {
    let env = crate::context::process_env();
    let (backend, herdr, state_dir, plugin_root) = match build_deps(&env) {
        Ok(deps) => deps,
        Err(error) => {
            eprintln!("herdr-kata: error: {}", error.message);
            return 1;
        }
    };
    if !std::io::stdout().is_terminal() {
        print_text(&backend, &state_dir);
        return 0;
    }
    run_tui(&backend, &herdr, &state_dir, &plugin_root)
}

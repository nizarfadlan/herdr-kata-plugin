//! herdr-kata: Herdr plugin that runs coding agents inside Kata Containers
//! microVMs. One subcommand per entrypoint: actions, the in-pane bridge, the
//! event hook and the microVMs overlay.

// Some modules expose surface that is only exercised on a live host
// (config keys, driver interface, agent adapters); silence dead-code lints
// for the parts unit tests do not reach.
#![allow(dead_code)]

mod action;
mod agents;
mod bridge;
mod config;
mod constants;
mod context;
mod describe;
mod errors;
mod events;
mod exec;
mod herdr;
mod kata;
mod lifecycle;
mod naming;
mod pane;
mod probes;
mod result;
mod run_action;
mod state;
mod util;
mod verify;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "herdr-kata",
    version,
    about = "Run coding agents inside Kata Containers microVMs from Herdr."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Arguments are passed through untouched: each subcommand parses its own.
#[derive(clap::Args)]
struct Passthrough {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the plugin action named by HERDR_PLUGIN_ACTION_ID.
    Action(Passthrough),
    /// Run inside a Herdr pane: `start`, `connect` or `shell`.
    Bridge(Passthrough),
    /// Handle the Herdr event named by HERDR_PLUGIN_EVENT.
    Events(Passthrough),
    /// Show the interactive microVMs overlay.
    Pane(Passthrough),
    /// Run a plugin action through Herdr and wait for its result (what scripts/run-action.sh does).
    RunAction(Passthrough),
    /// Run the escape probe checks; only meaningful inside a microVM.
    #[command(hide = true)]
    Probe(Passthrough),
    /// Print the action ids, config keys and constants as JSON.
    #[command(hide = true)]
    Describe,
}

/// Exits with the code of a finished command.
fn exit_with(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code.rem_euclid(256)).unwrap_or(1))
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Describe => {
            println!(
                "{}",
                serde_json::to_string_pretty(&describe::describe())
                    .expect("the description serialises")
            );
            ExitCode::SUCCESS
        }
        Command::Action(_) => exit_with(action::main(&context::process_env())),
        Command::Bridge(passthrough) => exit_with(bridge::run_bridge(&passthrough.args)),
        Command::Events(_) => exit_with(events::handle_event(&context::process_env())),
        Command::Pane(_) => exit_with(pane::run_pane()),
        Command::RunAction(passthrough) => exit_with(run_action::run_action_command(
            &passthrough.args,
            &context::process_env(),
        )),
        Command::Probe(passthrough) => probes::probe_main(&passthrough.args),
    }
}

# herdr-kata-plugin

A [Herdr](https://herdr.dev) plugin that runs coding agents inside **Kata Containers microVMs** — one microVM per Herdr pane.

> **v0.4.0.** One ephemeral Kata microVM per Herdr pane. Resource limits
> travel as Kata OCI annotations and are verified back from the container;
> egress is default-DROP via an in-guest firewall when the profile allowlists
> endpoints; the agent config is mounted read-only (`{agent_config}`); the
> guest entrypoint drops from root to the unprivileged `agent` user before
> the agent starts. See [docs/security.md](docs/security.md). Booting a real
> Kata microVM needs a host with `/dev/kvm` and Kata installed.

## Why

Herdr owns your terminals; this plugin decides *where the agent process lives*.
Instead of running the agent directly on your machine, each agent pane gets a
hardware-virtualized microVM (Kata Containers via containerd):

- the agent sees only its worktree and the config you grant it — not `~/.ssh`,
  not the Herdr socket, not D-Bus;
- every launch is verified from the host through containerd (Kata runtime,
  clean mounts);
- an escape probe proves the confinement on `doctor`.

## Requirements

- Linux with `/dev/kvm`
- Herdr 0.9+
- containerd + Kata Containers installed, runtime `io.containerd.kata.v2` registered (one-time root setup — see `kata/README.md`)
- your user can talk to `/run/containerd/containerd.sock`
- a coding agent available **inside the guest image** (Codex default; OpenCode, Claude Code, Pi and Oh My Pi also supported)

Run `doctor` first; it checks all of this and starts nothing.

## Get started

```bash
herdr plugin install nizarfadlan/herdr-kata-plugin
herdr plugin action invoke doctor --plugin kata.microvm
herdr plugin action invoke install-keybindings --plugin kata.microvm
```

Then open Herdr in a project, focus a pane and press **`prefix+Shift+a`**.

## Install details

`herdr plugin install` clones the repo and runs `scripts/install-binary.sh`
(the manifest's `[[build]]` step), which installs `bin/herdr-kata`:

1. it downloads the release binary matching `herdr-plugin.toml`'s `version`
   (`releases/download/v<version>/herdr-kata-<version>-<target>`) and checks
   its sha256 against the release's `SHA256SUMS` — no Rust toolchain needed;
2. when there is no such release (or no network), it falls back to
   `cargo build --release`.

The guest images work the same way: when neither `config.toml` nor the
profile sets `image`, the plugin pulls
`ghcr.io/nizarfadlan/herdr-kata-<agent>:<version>` (published automatically
on every release tag). Override with a pinned image of your own any time.

## Key bindings

| Chord | Action | Does |
|---|---|---|
| `prefix+Shift+a` | `start-agent` | Start the agent in a new Kata microVM |
| `prefix+Shift+o` | `microvms` | Overlay: every agent microVM |
| `prefix+Shift+b` | `reconnect` | Resume in a fresh microVM |
| `prefix+Shift+s` | `open-shell` | Shell in a microVM with the agent's policy |

## Hardening

See [docs/security.md](docs/security.md) for the full model. In short:

- one ephemeral microVM per pane, Kata runtime verified on every launch;
- only the worktree is mounted writable (`/work`); the agent config is
  mounted read-only when `agent_config_dir` is set in `config.toml`;
- resource limits (`vcpus`, `memory_mb`) travel as Kata OCI annotations and
  are verified back from the container;
- egress: without CNI the microVM is loopback-only; where the guest does
  get a network, the profile's `egress_allowlist` is enforced default-DROP
  by the in-guest firewall;
- the guest entrypoint drops from root to the unprivileged `agent` user
  before the agent starts.

## Verification

Against a real containerd 2.4.1 (a dev VM without `/dev/kvm`, so the
Kata runtime itself could not boot there):

- every `ctr` flag/subcommand the driver uses exists (`run`/`create`/`info`/
  `list`/`rm`, `tasks kill`/`rm`, `images list -q`, `--runtime`/`--rm`/
  `-t`/`--mount`/`--label`/`--cwd`/`--address`/`--namespace`);
- the `c info` JSON shape the verifier parses (`Runtime.Name`,
  `Spec.mounts[].source`);
- `verify-microvm` honestly fails the runtime check on a runc container and
  passes mount checks; `stop` really destroys the container;
- `doctor` reports each prerequisite individually;
- `probe --json` emits structured results (fails on a non-guest host, as
  designed — the checks are tripwires for misconfigured profiles).

## Configuration

`herdr plugin config-dir kata.microvm` prints the config directory. Create
`config.toml` there (all keys optional):

```toml
agent = "codex"                  # codex (default) | opencode | claude | pi | omp
kata_bin = "ctr"                 # ctr | nerdctl
containerd_socket = "/run/containerd/containerd.sock"
containerd_namespace = "herdr-kata"
kata_runtime = "io.containerd.kata.v2"
image = ""                       # override the guest image
profiles_dir = "profiles"
agent_args = []
```

VM policy lives in `profiles/*.json` (vCPU, memory, image, mounts, egress
allowlist).

## Layout

- `herdr-plugin.toml` — the Herdr plugin manifest
- `rust/src/` — the `herdr-kata` binary (one binary, no runtime deps)
- `bin/run.sh` — dispatcher Herdr calls
- `scripts/` — binary installer (release + sha256, fallback `cargo build`), keybindings, action runner
- `profiles/` — microVM profiles per agent
- `kata/` — host setup docs + example Kata config
- `guest/` — guest image Dockerfiles (agent + CA certs)
- `docs/design.md` — the full design doc

## Development

```bash
cargo build
cargo test
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```

## License

Apache-2.0. See `LICENSE`.

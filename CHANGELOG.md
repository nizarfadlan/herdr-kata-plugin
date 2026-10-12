# Changelog

## 0.4.0 (2026-10-12)

Hardening, plus three new agent adapters:

- New adapters: `claude` (Claude Code, resume via `--continue`), `pi` (pi.dev,
  resume semantics not yet verified), `omp` (Oh My Pi, resume via `--resume`);
  each with a Kata profile (`profiles/herdr-<agent>-kata.json`) and a guest
  Dockerfile (`guest/Dockerfile.<agent>`).

- Resource limits: the profile's `vcpus`/`memory_mb` become Kata OCI
  annotations (`io.katacontainers.config.hypervisor.default_vcpus` /
  `default_memory`, passed via `ctr run --annotation`); `verify-microvm`
  gains a `resource-limits` check reading them back from the container.
- Egress policy: the profile's `egress_allowlist` is enforced by an
  in-guest firewall (`guest/egress.sh`, default-DROP + allowlist) when
  non-empty; the plugin then passes `HERDR_EGRESS_ALLOW` and grants
  `NET_ADMIN` (`--cap-add`). Empty allowlist: the firewall stays off
  (without CNI the microVM is loopback-only anyway).
- Agent config mount: the profile's `{agent_config}` template now resolves
  to `agent_config_dir` from `config.toml` (read-only); the mount is
  skipped when unconfigured. `{worktree}` resolves to the focused pane's
  worktree; unknown templates are rejected.
- Guest images: new root entrypoint applies the firewall, then drops to the
  unprivileged `agent` user via `setpriv` before the agent starts.
- New `docs/security.md` with the enforcement matrix; `doctor` reports the
  egress policy, resources and the agent config mount.
- CI: GitHub Actions runs `cargo fmt --check`, `clippy -D warnings`,
  `cargo test` and `sh -n` over the scripts.

## 0.3.0 (2026-10-12)

The interactive `microvms` overlay and `run-action`:

- New `pane/` module (ratatui): a live table of every tracked microVM
  (state, pane, microVM, agent, runtime) with details (`enter`), help (`?`),
  jump to the pane (`g`, via workspace/tab focus), verify (`v`), stop (`x`,
  asks first), prune (`p`), clean up all (`a`, asks first), refresh (`r`).
  Without a terminal it prints the table as plain text.
- `microvms` action opens the overlay via
  `herdr plugin pane open --plugin kata.microvm --entrypoint microvms --focus`.
- `run-action` fully implemented: invokes the action through
  `herdr plugin action invoke`, waits for this invocation's log entry
  (matched by log id), prints the result line and stderr; exits 0/1/2.
- Herdr client: `open_plugin_pane`, `pane_info`, `focus_pane`; transport
  failures are now distinguished from "the command reported an error".
- Tests: 51 pass, including ratatui `TestBackend` render tests, key-handling
  tests and a `run-action` end-to-end run against a fake Herdr CLI.

## 0.2.0 (2026-10-12)

The agent lifecycle works against containerd:

- `kata.rs`: full `CtrBackend` (every flag verified against containerd 2.4.1),
  `c info` JSON parsing, `destroy`, `image_exists`, `run_argv`.
- New modules: `exec.rs` (timeout-guarded CLI runner), `herdr.rs` (Herdr CLI
  client: split/run/get/list/close pane), `lifecycle.rs` (launch spec,
  pane wiring, wait-and-verify, stop), `naming.rs`, `state.rs` (atomic
  pane→microVM mappings), `util.rs` (shell quoting), `verify.rs`
  (container-exists, Kata runtime, clean mounts).
- Real `bridge`: execs `ctr run` with the pane's stdio inherited, cleans up
  the microVM and the mapping on exit.
- Actions wired: `start-agent`, `reconnect`, `open-shell`, `stop`, `info`,
  `verify-microvm`, `list-microvms`, `prune-mappings`, `forget-mapping`;
  `worktree.removed` stops affected microVMs.
- Real `probe`: guest-side tripwires (no host SSH, no containerd socket,
  Herdr socket unreachable), JSON output for `doctor`.
- `doctor` also checks the guest image.
- Verified live against containerd 2.4.1 (no `/dev/kvm` here, so the Kata
  runtime boot itself is still unverified — needs a KVM host).

## 0.1.0 (2026-10-11)

Plugin skeleton:

- `herdr-plugin.toml` manifest (`kata.microvm`), 12 actions, `worktree.removed` event hook, `microvms` overlay pane.
- `herdr-kata` Rust binary: `action`, `bridge`, `events`, `pane`, `probe`, `run-action`, `describe` subcommands.
- Working: `doctor` (checks /dev/kvm, containerd CLI + socket + API, Kata shim), `install-keybindings`, `describe`.
- Honest stubs for the agent lifecycle (`start-agent`, `reconnect`, `open-shell`, `stop`, `verify-microvm`, …).
- `scripts/install-binary.sh` (release download + sha256, fallback `cargo build`), `scripts/install-keybindings.sh`, `scripts/run-action.sh`, `bin/run.sh` dispatcher.
- VM profiles for Codex (default) and OpenCode; guest Dockerfiles; Kata host setup docs.

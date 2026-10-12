# herdr-kata-plugin — Design Doc

- **Date:** 2026-10-11 (translated to English 2026-10-12; revised 2026-10-12)
- **Status:** Implemented and released as v0.4.0 on [`nizarfadlan/herdr-kata-plugin`](https://github.com/nizarfadlan/herdr-kata-plugin) (public).
- **Context:** [herdrdev/herdr](https://github.com/herdrdev/herdr) — *"the runtime your coding agents live on"*, a Rust terminal-multiplexer/agent runtime with a plugin system — and plugins like [herdr-nono-plugin](https://github.com/schemaitat/herdr-nono-plugin) (a herdr plugin that runs OpenCode inside **nono** sandboxes). This project builds the same kind of plugin, but with **Kata Containers microVMs** as the isolation layer.

## 1. The herdr plugin contract (facts from the official docs)

Read from `docs/next/website/src/content/docs/plugins.mdx` of the herdr repo (cloned 2026-10-11):

- A plugin is a directory with a **`herdr-plugin.toml`** manifest plus argv commands Herdr can launch. There is no special SDK — **the entire Herdr CLI is the plugin API**.
- The manifest declares: `id`, `name`, `version`, `min_herdr_version`, `platforms`, `[[build]]` (runs at install), `[[actions]]` (id, title, contexts, command argv), `[[events]]` (on, command), `[[panes]]` (id, placement, command), `[[keys.command]]` (keybinding → plugin action), `[[link_handlers]]`.
- Plugin commands run with the plugin directory as CWD and env: `HERDR_SOCKET_PATH`, `HERDR_BIN_PATH` (use this to call back into Herdr — portable), `HERDR_ENV=1`, `HERDR_PLUGIN_ID/ROOT/CONFIG_DIR/STATE_DIR/CONTEXT_JSON`, `HERDR_WORKSPACE_ID/TAB_ID/PANE_ID`; actions also get `HERDR_PLUGIN_ACTION_ID`, event hooks get `HERDR_PLUGIN_EVENT` (+`_JSON`).
- User config lives in `HERDR_PLUGIN_CONFIG_DIR`, runtime state in `HERDR_PLUGIN_STATE_DIR` (never in the plugin root — GitHub-installed roots are managed by Herdr).
- Install: `herdr plugin install owner/repo[/subdir]` (GitHub shorthand, build runs, preview shown); local development via `herdr plugin link /path`. The marketplace auto-indexes public GitHub repos tagged with the `herdr-plugin` topic that contain a `herdr-plugin.toml`.

## 2. What this mirrors from herdr-nono-plugin (architecture template)

The structure of `herdr-nono-plugin` was studied directly from source (Apache-2.0). This project keeps its patterns:

| Aspect | herdr-nono-plugin | herdr-kata-plugin |
|---|---|---|
| Manifest | `herdr-plugin.toml`, id `nono.sandbox` | `herdr-plugin.toml`, id **`kata.microvm`** |
| Binary | one Rust binary `herdr-nono`, no Node.js at runtime | one Rust binary **`herdr-kata`** |
| Subcommands | `action` / `bridge` / `events` / `pane` / `probe` / `describe` | same (§5) |
| Binary install | `scripts/install-binary.sh`: downloads the release + sha256 check, falls back to `cargo build` | same |
| Bridge | runs **inside the Herdr pane**, launches the sandboxed agent with the pane's stdio | same — the bridge launches the agent **inside a Kata microVM**; stdio is passed through (Herdr never needs to know about the VM) |
| Isolation driver | `nono.rs`: thin argv wrapper around the `nono` CLI (never a shell string), per-call timeouts | **`kata.rs`**: thin argv wrapper around `ctr`/`nerdctl`; a native containerd-gRPC backend is future work (see §4) |
| Policy | `profiles/*.json` (nono profiles: server + client) | **`profiles/*.json`**: Kata profiles — vCPU/mem, image, mounts, egress allowlist (see §6) |
| Verification | `verify.rs` via `/proc`: every process confined, the TUI talking to its own server | `verify.rs` via **containerd**: container running with the Kata runtime, the agent process as the container init, no sensitive host mounts, escape probe fails |
| Overlay | `pane/` (ratatui): agents + sandboxes + verification | same, titled "microVMs" |
| Keybindings | `prefix+Shift+a/o/b/s` via the `install-keybindings` action | same chords (used when free) |
| Actions | doctor, start-agent, reconnect, open-shell, stop, info, verify, prune, overlay… | same, `sandboxes` → **`microvms`** |
| Event | `worktree.removed` → stop agents whose worktree was removed | same |

## 3. Key differences: nono → Kata Containers

1. **Granularity:** the nono plugin uses **two sandboxes per agent** (OpenCode server + TUI client — specific to OpenCode's architecture). The Kata plugin uses **one microVM per agent pane** — the whole agent (TUI + everything it runs) lives inside a single VM. Simpler and agent-agnostic.
2. **Driver:** nono is a session-oriented CLI (`nono run`, `nono ps --json`, `nono stop`) that runs as an ordinary user (user namespaces). Kata has no session CLI — access goes through **containerd** (runtime `io.containerd.kata.v2` → shim `containerd-shim-kata-v2`; verified from the containerd-kata docs). Consequences:
   - **containerd + Kata must be installed on the host** (one-time root step; see `kata/README.md`).
   - The plugin needs access to `/run/containerd/containerd.sock` (user in the socket group / sudo — documented, like Docker-based plugins).
   - **`/dev/kvm`** is required on the host (hard prerequisite; `doctor` checks it).
3. **Verification:** nono is verified via `/proc` from the host (supervisor outside the sandbox). Kata is verified via the **containerd API**: the container exists + `runtime.name == io.containerd.kata.v2` + OCI spec inspection (mounts, netns) + an **escape probe** (attempts to reach host-only paths / the Herdr socket from inside the VM → must fail).
4. **Networking:** nono has an egress proxy + `af_unix_mediation`. With Kata, each VM gets its own netns; the policy is **default DROP with an egress allowlist** to model-provider endpoints, enforced by an in-guest firewall (`guest/egress.sh`) when the profile's `egress_allowlist` is non-empty. Without CNI the microVM is loopback-only and the firewall stays off.
5. **Image:** nono uses a "profile pack" (`nono pull`). Kata uses an **OCI image** containing the agent binary (codex/opencode) + CA certs — `guest/Dockerfile.*` in the repo, version-pinned, `doctor` ensures the image exists (pull/build).

## 4. Architecture

```
Herdr (server, host)
  │ new pane (split) — Herdr only sees an ordinary process with stdio
  ▼
bin/run.sh bridge start …          # inside the pane
  ▼
herdr-kata bridge start --pane-id … --plugin-root …
  │ 1. read config (HERDR_PLUGIN_CONFIG_DIR) → pick agent adapter + profile
  │ 2. kata.rs: ctr run --runtime io.containerd.kata.v2 -t --rm
  │         --mount … <guest-image> <id> <agent-cmd…>
  │    pane stdio ↔ agent process inside the microVM (byte-identical)
  │ 3. verify: containerd inspect → runtime=kata, mounts clean
  │ 4. record the pane→container mapping in HERDR_PLUGIN_STATE_DIR
  ▼
Kata microVM (1 per agent pane, ephemeral)
  ┌──────────────────────────────────────────────┐
  │ guest: agent (codex/opencode) + CA certs      │
  │ mounts: project worktree (rw), just enough   │
  │         agent config (ro), /tmp              │
  │ network: own netns; egress default-DROP      │
  │          when the profile allowlists         │
  │          endpoints (in-guest firewall)       │
  │ ABSENT: host ~/.ssh, Herdr socket, D-Bus,    │
  │         any other credentials                │
  └──────────────────────────────────────────────┘
```

**Driver backend:** `CtrBackend`, an argv wrapper around `ctr` (a 1:1 mirror of the `nono.rs` pattern — "direct argv, never a shell string", per-call timeouts). A `ContainerdGrpcBackend` using the official `containerd-client` crate (containerd/rust-extensions, gRPC; needs `protoc` at build time) behind the same `KataDriver` trait is future work. Rationale: the example's pattern is proven, `ctr` always exists where containerd does, and migrating to gRPC later changes no other module.

## 5. Manifest & subcommands

`herdr-plugin.toml` (id **`kata.microvm`**, name "Kata MicroVMs", `platforms = ["linux"]`, `min_herdr_version = "0.9.0"` following the example):

- `[[build]]` → `sh scripts/install-binary.sh`
- actions: `doctor`, `install-keybindings`, `start-agent`, `reconnect`, `open-shell`, `stop`, `info`, `verify-microvm`, `prune-mappings`, `microvms` (overlay), `list-microvms`, `forget-mapping` — all `command = ["sh", "bin/run.sh", "action"]`, with contexts like the example (`start-agent`: workspace+pane; `doctor`: global+workspace+pane).
- `[[events]] on = "worktree.removed"` → `["sh", "bin/run.sh", "events"]`
- `[[panes]] id = "microvms"`, `placement = "overlay"`, `command = ["sh", "bin/run.sh", "pane"]`
- `[[keys.command]]`: `prefix+Shift+a` → `kata.microvm.start-agent`, `prefix+Shift+o` → `kata.microvm.microvms`, `prefix+Shift+b` → `kata.microvm.reconnect`, `prefix+Shift+s` → `kata.microvm.open-shell` (used when not already bound; `install-keybindings` skips existing ones — the example's pattern).

The `herdr-kata` binary's subcommands (mirroring the example's `main.rs`): `action` (reads `HERDR_PLUGIN_ACTION_ID`), `bridge <start|connect|shell>`, `events` (reads `HERDR_PLUGIN_EVENT`), `pane` (ratatui overlay), `probe` (escape probe; only meaningful inside the VM), `describe` (JSON: action ids, config keys — for tooling).

## 6. Rust modules (mirroring the example; only the driver changes)

```
rust/src/
  main.rs, action.rs, agents.rs, bridge.rs, config.rs, constants.rs,
  context.rs, describe.rs, errors.rs, events.rs, exec.rs, herdr.rs,
  kata.rs, lifecycle.rs, naming.rs, pane/{mod,app,model,ui}.rs,
  probes.rs, result.rs, run_action.rs, state.rs, util.rs, verify.rs
```

- **`kata.rs`** (replaces `nono.rs`): a `KataDriver` trait + `CtrBackend`; `classify_failure` from `ctr` output; normalisation of `ctr` listings into `MicroVm { id, runtime, status, … }`; an argv builder for `ctr run` (runtime flag, mounts, `--rm`, `-t`).
- **`agents.rs`**: generic adapters — `{ title, command, default_args, resume_args, profile, herdr_detection_kind }`. Built in: **`codex`** (default), **`opencode`**, **`claude`**, **`pi`**, **`omp`** (single-VM mode; no client/server split — a simplification vs. the example). Custom agents via config (the example's pattern).
- **`profiles/`**: `profiles/herdr-codex-kata.json`, `profiles/herdr-opencode-kata.json` — `{ vcpus, memory_mb, image, mounts, egress_allowlist }`. (Not nono profiles; the plugin's own format, resolved by `kata.rs`.)
- **`verify.rs`**: host-side checks via containerd — the container exists and uses the Kata runtime; the OCI spec does not mount sensitive host paths (`~/.ssh`, `/run/herdr`, …); the escape probe from inside the VM fails to reach the host.
- **`probes.rs`**: runs inside a throwaway VM: tries to read a host-only marker file, tries `connect()` to the visible Herdr socket, tries egress to a non-allowlisted host — all must fail; JSON output for `doctor`.
- **`lifecycle.rs`**: start/stop/verify/shell — same shape as the example, only the driver differs.
- **`config.rs`**: config keys (mirroring the example): `agent` (codex|opencode|custom), `kata_bin`/`containerd_socket`, `image`, `profiles_dir`, `agentArgs`, etc.

## 7. Main workflows

- **`doctor`**: checks `herdr ≥0.9`, `/dev/kvm`, `ctr` + containerd reachability, the Kata runtime, the guest image, then an **escape probe in a throwaway VM** with the active profile. Starts no agent.
- **`start-agent`** (prefix+Shift+a): from the pane context (worktree), split a pane → `bridge start` → `ctr run --runtime io.containerd.kata.v2` with the worktree mounted rw → verify → record the mapping. Quitting the agent removes the VM (`--rm`); the mapping is cleaned up.
- **`reconnect`**: relaunches with the adapter's `resume_args` in a new VM (VMs are ephemeral — resume means a new session with the agent's resume arguments, not the same VM; documented explicitly).
- **`open-shell`**: an interactive shell in a new VM with the same profile and worktree.
- **`microvms` overlay**: lists agents + VMs + verification status; keys: details, jump to pane, verify, stop, prune, clean up all.
- **`worktree.removed`**: stops that worktree's VMs and forgets their mappings.

## 8. Security (parity with the example, adapted to VMs)

- **Deny by default:** Kata VMs get no host files except the worktree (writable), the agent config (read-only) and what the profile lists; egress is allowlist-only (model-provider endpoints) + DROP everything else when the profile's `egress_allowlist` is non-empty.
- **Host sockets blocked:** the Herdr socket (`~/.config/herdr/herdr.sock`), D-Bus, ssh-agent, docker are neither mounted into the VM nor reachable over the network (the example blocks via `af_unix_mediation`; with Kata the blocking is structural — no mounts, separate netns).
- **External verification:** every launch is verified from the host via containerd; if verification fails → the agent is stopped (the example's pattern).
- **NOT covered** (stated honestly, mirroring the example's Security section): the project worktree itself (the agent legitimately needs to write there), the shared agent config that gets mounted, and guest-image supply-chain security (pin versions + sha256 at install).
- **Trust:** the plugin runs as your user with containerd-socket access — equivalent in power to the Docker group; documented in the README, and `doctor` warns about it.

## 9. Repository layout (`nizarfadlan/herdr-kata-plugin`)

```
herdr-kata-plugin/
├── herdr-plugin.toml
├── Cargo.toml / Cargo.lock
├── rust/                      # Rust workspace (src/ as in §6)
├── bin/run.sh                 # dispatcher: action|bridge|events|pane → bin/herdr-kata
├── scripts/
│   ├── install-binary.sh      # release download + sha256, fallback cargo build
│   ├── install-keybindings.sh
│   └── run-action.sh
├── profiles/                  # per-agent VM profiles (JSON)
├── kata/
│   ├── configuration-agent.toml.example  # example Kata host config (info; needs root)
│   └── README.md
├── guest/
│   ├── Dockerfile.codex
│   ├── Dockerfile.opencode
│   ├── egress.sh              # in-guest egress firewall (default-DROP + allowlist)
│   └── entrypoint.sh          # applies the firewall, drops to the `agent` user
├── docs/                      # design.md, security.md
├── .github/workflows/ci.yml   # fmt, clippy, test, sh -n
├── justfile, LICENSE, NOTICE
└── README.md
```

GitHub topic `herdr-plugin` for marketplace indexing.

## 10. Current state (v0.4.0)

- **Host prerequisites:** `/dev/kvm` present; containerd + Kata installed with `io.containerd.kata.v2` registered; the user can run `ctr` (socket group). `doctor` checks all of these.
- **Implemented:** `kata.rs` `CtrBackend` (every `ctr` flag verified against containerd 2.4.1); the full agent lifecycle (`start-agent`, `reconnect`, `open-shell`, `stop`, `info`, `verify-microvm`, `list-microvms`, `prune-mappings`, `forget-mapping`); the `microvms` overlay (ratatui live table + details/jump/verify/stop/prune/clean-all, plain-text fallback); `worktree.removed` handling; `run-action` (Herdr action invocation + log polling); hardening — resource limits as Kata OCI annotations (verified back), egress default-DROP via the in-guest firewall, `{agent_config}` read-only mount, guest entrypoint dropping to the `agent` user; `docs/security.md`; CI (`cargo fmt --check`, `clippy -D warnings`, `cargo test`, `sh -n`).
- **Verified:** against containerd 2.4.1 — `ctr` argv, `c info` JSON parsing, annotation round-trip, bridge argv, egress script logic, `doctor` output, `verify-microvm` (honestly fails the runtime check on a runc container), `stop` (destroys the container). 54 unit tests pass. Not verifiable without a KVM host: actual Kata microVM boot and the interactive TUI inside a Herdr pane.
- **Future work:** native gRPC `ContainerdGrpcBackend`; additional agent adapters; pinned guest images (sha256); `cargo deny`.

## 11. Risks & unknowns

| # | Risk / unknown | Mitigation |
|---|---|---|
| R1 | Containerd-socket access needs privilege (group/root) — install UX is less smooth than nono's | `doctor` + README explain the one-time step; the Docker pattern is well known |
| R2 | `/dev/kvm` missing (VPS without nested virt) | `doctor` checks this — a project stopper if it fails; cheap to check. |
| R3 | Interactive TUIs (opencode/codex) over `ctr run -t` inside a herdr pane | Fallback = PTY allocation in the bridge if needed |
| R4 | The correct `resume_args` for codex | Verified during implementation (`codex --help`); never guessed |
| R5 | How to check Kata runtime registration via `ctr` | `containerd config dump`, or a probe run |
| R6 | Guest image pull size/latency and per-agent VM boot latency | Documented |
| R7 | herdr minimum version — the example targets 0.9.x; APIs may change | Pin `min_herdr_version`, test against the user's installed version |

## 12. Decisions (all taken)

1. **Repo**: `nizarfadlan/herdr-kata-plugin` (public).
2. **Default agent**: **Codex** (the user's stack); OpenCode as the second adapter, both single-VM.

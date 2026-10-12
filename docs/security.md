# Security model

How the plugin confines a coding agent, and where each control is enforced.
The model is layered: no single control is trusted alone.

## Boundaries

| Control | Enforced by | Notes |
|---|---|---|
| Hardware isolation | Kata microVM (`io.containerd.kata.v2`) | One ephemeral microVM per agent pane; the VM is discarded when the agent exits. |
| Worktree grant | Plugin (`--mount type=bind,…,options=rbind:rw`) | Only the focused pane's worktree is mounted, at `/work`. Nothing else from the host is mounted. |
| Agent config | Plugin (`{agent_config}` template, `ro`) | The host dir from `agent_config_dir` is mounted read-only at the agent's config path. Skipped when unconfigured; never writable. |
| Forbidden mounts | `verify-microvm` (`clean-mounts`) | `never_mount` (`~/.ssh`, `/run/herdr`, `/run/dbus`, `/run/containerd`) must not appear among the container's mount sources. A violation stops the agent. |
| Kata runtime | `verify-microvm` (`kata-runtime`) | The container's runtime name must equal the configured Kata runtime. |
| Resource limits | Kata annotations + `verify-microvm` (`resource-limits`) | `vcpus`/`memory_mb` from the profile become `io.katacontainers.config.hypervisor.default_vcpus` / `default_memory` OCI annotations; the verifier checks they are present on the container. |
| Non-root agent | Guest image entrypoint | The entrypoint starts as guest-root only to apply the firewall, then `setpriv`s to the unprivileged `agent` user before the agent starts. Guest images must keep this shape. |
| Egress | Guest firewall (`guest/egress.sh`) | See below. |

## Egress

The network story has two layers:

1. **Plain `ctr run` without CNI gives the container loopback only.** With no
   `--net-host` and no CNI configuration on the host, the microVM has no
   route to the outside world. That is the default, and it is already
   default-deny.
2. **On hosts where the guest does get a network** (CNI configured, or the
   admin opted into `--net-host`), the in-guest firewall enforces the
   profile's `egress_allowlist`: `egress.sh` sets `OUTPUT` to `DROP` and
   allows only the listed `host:port` pairs (TCP), plus loopback and
   established connections. Entries are resolved once at boot and pinned to
   IPs. Malformed entries are ignored with a warning.

Mechanics:

- The plugin passes the allowlist as `HERDR_EGRESS_ALLOW` and grants
  `NET_ADMIN` (`--cap-add`) **only** when the allowlist is non-empty.
- An empty allowlist means the firewall stays off: the agent then relies on
  layer 1 (loopback-only) or on whatever the host provides. `doctor` prints
  the effective policy so this is never a silent assumption.
- The firewall needs `iptables` in the guest image (both Dockerfiles install
  it) and a guest kernel with conntrack.

## What the plugin does not do

- It does not audit the agent's behavior inside the VM; observability
  (per-session tool/file/network telemetry) is a separate track.
- It does not protect the host from a malicious containerd or a malicious
  Kata runtime; those are trusted components.
- `verify-microvm` checks host-visible state (runtime, annotations, mounts).
  It cannot inspect guest iptables rules; the egress posture is enforced at
  guest boot and reported by `doctor`.
- Secrets (provider keys) reach the guest only through the read-only agent
  config mount. There is no other secret channel, and the plugin never logs
  their values.

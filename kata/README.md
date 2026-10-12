# Kata host configuration for herdr-kata-plugin (example)

This file documents the **one-time host setup** (needs root) so that the
plugin's `kata.microvm` driver can launch microVMs through containerd.

> Do not copy blindly: paths and versions depend on how Kata was installed
> ( distro packages vs. `kata-deploy` vs. static tarball ).

## 1. Install Kata Containers

Follow the official Kata Containers installation guide for your distro, or:

```bash
# Example (Debian/Ubuntu, Kata 3.x):
# see https://github.com/kata-containers/kata-containers for the current steps
```

What must exist afterwards:

- `containerd-shim-kata-v2` on `PATH` (the plugin's `doctor` checks this)
- `/dev/kvm` present (the plugin's `doctor` checks this)

## 2. Register the runtime in containerd

`/etc/containerd/config.toml` (containerd 1.7.x shown; for 2.x the plugin path
prefix is `io.containerd.cri.v1.runtime`):

```toml
version = 2
[plugins."io.containerd.grpc.v1.cri".containerd.runtimes.kata]
  runtime_type = "io.containerd.kata.v2"
  [plugins."io.containerd.grpc.v1.cri".containerd.runtimes.kata.options]
    ConfigPath = "/opt/kata/share/defaults/kata-containers/configuration.toml"
```

The plugin asks containerd for exactly `io.containerd.kata.v2`
(`kata_runtime` in `config.toml`).

## 3. Hypervisor sizing (optional)

The plugin's profiles (`profiles/*.json`) declare `vcpus`/`memory_mb` per
agent class, passed as Kata OCI annotations
(`io.katacontainers.config.hypervisor.default_vcpus` /
`default_memory`). Keep the Kata host defaults reasonable as a fallback.

## 4. Socket access

The plugin runs as your user and talks to `/run/containerd/containerd.sock`.
Give your user access (e.g. add it to the group owning the socket) — the same
trade-off as the Docker group. `doctor` reports when the socket is missing.

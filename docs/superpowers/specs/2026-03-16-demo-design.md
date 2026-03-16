# steep demo: Repeatable End-to-End Demonstration

## Overview

A runnable demo that exercises both `steep cloud-init` and `steep container` end-to-end. Each example builds a confidential VM image running caddy serving a static success page. `bin/demo` runs both in parallel via tmux; each example can also be run independently.

## File Layout

```
examples/
├── cloud-init/
│   ├── run.sh          # full pipeline: base → cloud-init → steep run --port-forward 8080:80
│   ├── meta-data       # cloud-init instance-id + hostname
│   └── user-data       # installs caddy via apt repo, writes Caddyfile + index.html
└── container/
    ├── run.sh          # full pipeline: base → container → steep run --port-forward 8081:80
    ├── Dockerfile      # FROM caddy:latest + baked-in Caddyfile + index.html
    ├── Caddyfile       # :80 { root * /usr/share/caddy; file_server }
    └── index.html      # static success page

bin/
└── demo               # opens tmux session, runs each run.sh in a pane, stays attached
```

## `steep run` Changes: `--port-forward`

`RunArgs` gains an optional, repeatable flag:

```
--port-forward HOST:GUEST   # e.g. --port-forward 8080:80
```

Multiple forwards are supported (e.g. `--port-forward 8080:80 --port-forward 8443:443`).

In `QemuArgs::to_args()`, all forwards are combined into a single `-netdev user` device:

```
-netdev user,id=net0,hostfwd=tcp::8080-:80,hostfwd=tcp::8443-:443
-device virtio-net-pci,netdev=net0
```

The `-device virtio-net-pci,netdev=net0` line appears once regardless of how many forwards are specified. If no `--port-forward` flags are given, no network device is added (preserving the current behaviour).

## Example: `examples/cloud-init/`

### `meta-data`

```yaml
instance-id: steep-demo-cloud-init
local-hostname: steep-demo
```

### `user-data`

Installs caddy from the official apt repo, writes a Caddyfile and success page, starts caddy on port 80.

```yaml
#cloud-config
packages:
  - debian-keyring
  - debian-archive-keyring
  - apt-transport-https
  - curl

write_files:
  - path: /var/www/html/index.html
    content: |
      <!DOCTYPE html>
      <html><head><title>steep demo</title></head>
      <body><h1>steep demo</h1>
      <p>Served by caddy inside a confidential VM built with steep (cloud-init).</p>
      </body></html>
  - path: /etc/caddy/Caddyfile
    content: |
      :80 {
          root * /var/www/html
          file_server
      }

runcmd:
  - curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  - curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list
  - apt-get update
  - apt-get install -y caddy
  - systemctl enable --now caddy
```

### `run.sh` pipeline

1. Parse `--force` flag
2. Resolve host kernel: `/boot/vmlinuz-$(uname -r)` and initrd: `/boot/initrd.img-$(uname -r)`
3. Download base image to `~/.local/share/steep/ubuntu-resolute-amd64v3.img` from `https://cloud-images.ubuntu.com/resolute/current/resolute-server-cloudimg-amd64v3.img` (skip if already cached — never re-downloaded with `--force`)
4. `steep base` → `output/demo/base/` (skip if `output/demo/base/base.raw` exists and no `--force`)
5. `steep cloud-init examples/cloud-init` → `output/demo/cloud-init/` (skip if `output/demo/cloud-init/manifest.json` exists and no `--force`)
6. Print URL: `http://localhost:8080`
7. `sudo steep run --port-forward 8080:80 output/demo/cloud-init` (foreground)

## Example: `examples/container/`

### `index.html`

```html
<!DOCTYPE html>
<html><head><title>steep demo</title></head>
<body><h1>steep demo</h1>
<p>Served by caddy inside a confidential VM built with steep (container).</p>
</body></html>
```

### `Caddyfile`

```
:80 {
    root * /usr/share/caddy
    file_server
}
```

### `Dockerfile`

```dockerfile
FROM caddy:latest
COPY index.html /usr/share/caddy/index.html
COPY Caddyfile /etc/caddy/Caddyfile
```

### `run.sh` pipeline

1. Parse `--force` flag
2. Resolve host kernel and initrd (same as cloud-init)
3. Download base image (same cache, same skip logic)
4. `steep base` → `output/demo/base/` (shared with cloud-init, same skip logic)
5. `podman build -t steep-demo-container:latest examples/container/`
6. `steep container steep-demo-container:latest` → `output/demo/container/` (skip if `output/demo/container/manifest.json` exists and no `--force`)
7. Print URL: `http://localhost:8081`
8. `sudo steep run --port-forward 8081:80 output/demo/container` (foreground)

## `bin/demo` Orchestrator

1. Parse `--force`, pass through to both `run.sh` calls
2. Check `tmux` is available (fail with clear message if not)
3. Create tmux session `steep-demo` (or attach if it already exists)
4. Split into two panes: left runs `examples/cloud-init/run.sh [--force]`, right runs `examples/container/run.sh [--force]`
5. Attach to the session

## Shared Inputs

| Input | Value |
|-------|-------|
| Kernel | `/boot/vmlinuz-$(uname -r)` |
| Initrd | `/boot/initrd.img-$(uname -r)` |
| Firmware | `~/.local/share/steep/OVMF.fd` |
| Base image URL | `https://cloud-images.ubuntu.com/resolute/current/resolute-server-cloudimg-amd64v3.img` |
| Base image cache | `~/.local/share/steep/ubuntu-resolute-amd64v3.img` |
| Service port (cloud-init) | 8080 |
| Service port (container) | 8081 |

## Output Artifacts

```
output/demo/
├── base/
│   └── base.raw
├── cloud-init/
│   ├── disk.qcow2
│   ├── guest.igvm
│   ├── uki.efi
│   └── manifest.json
└── container/
    ├── disk.qcow2
    ├── guest.igvm
    ├── uki.efi
    └── manifest.json
```

## Idempotency

- Base image download: never re-downloaded (cached by filename)
- `steep base`: skipped if `output/demo/base/base.raw` exists, unless `--force`
- `steep cloud-init`: skipped if `output/demo/cloud-init/manifest.json` exists, unless `--force`
- `steep container`: skipped if `output/demo/container/manifest.json` exists, unless `--force`
- `podman build`: always re-run (fast, and ensures the local image is current)
- `--force` removes the relevant output directory before rebuilding

## QEMU Launch

The demo uses SEV-SNP hardware with `sudo`. The `--port-forward` flag on `steep run` adds user-mode networking to the QEMU invocation. Each VM runs in the foreground of its tmux pane (or terminal when run standalone).

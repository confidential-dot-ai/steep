# Steep — Notes & Documentation TODO

## Threat Model Documentation Needed

### What attestation guarantees (and doesn't)

The SNP launch digest proves the **initial state** of the guest image at build time:
- The disk image content (via dm-verity roothash)
- The kernel + initrd + cmdline (via UKI measurement)
- The cloud-init config files (measured in the verity root)

It does **not** prove runtime state. The overlayfs design means:
- Cloud-init config is attested (it's in the verified lower layer)
- Cloud-init execution results (written files, installed packages) land on the ephemeral tmpfs upper layer — unattested
- Any root process can modify runtime state via the overlay without invalidating the roothash
- Runtime integrity (proving "the system is still running exactly what was measured") is an orthogonal problem (IMA/EVM, runtime attestation) and out of scope for steep

In short: attestation proves "this VM was told to do X", not "X actually happened correctly at runtime."

### Overlay security properties

The tmpfs upper layer is mounted with `nosuid,nodev`:
- `nosuid`: setuid/setgid bits on binaries written to the upper layer are ignored — prevents privilege escalation via planted setuid binaries
- `nodev`: device node creation is blocked on the upper layer — prevents access to host devices via crafted device nodes
- Legitimate setuid binaries (sudo, passwd) on the verified lower layer are unaffected

### Autologin + serial console

`Autologin=true` in mkosi.conf enables passwordless root on ttyS0. In the SNP threat model, the host controls the serial port. This gives the host an authenticated root session — acceptable for dev/debug but must be disabled for production images. Document as a configuration knob.

### Bake mode trust model

`--bake` executes cloud-init user-data as root on the build machine inside a chroot with bind-mounted `/dev`. This means:
- The user-data is trusted — it runs with full host device access
- Network access is available (DNS set to 1.1.1.1/8.8.8.8)
- Bake mode is for trusted, operator-authored configs only
- Bake mode is NOT the reproducibility target — boot-time cloud-init is

**Decision (AUDIT-3):** Not sandboxing the bake chroot further right now. The `/dev` bind-mount is not the real risk — `runcmd` execution as root is. Proper sandboxing (systemd-nspawn, bubblewrap, capability dropping, seccomp) is significant effort for a non-production path. Instead: add a CLI warning when `--bake` is used so operators know they're running user-data as root on the build host. Revisit if bake becomes a production path.

### Bake mode: what works and what doesn't

Bake mode runs cloud-init in a chroot — not a booted system. There is no running systemd, no proper user database plumbing, no `/dev/urandom` for key generation. This limits which cloud-init modules work.

**Works reliably in bake mode:**
- `write_files` — writes bytes to paths, no system dependencies
- `packages` — apt works in the chroot (network is available)
- `runcmd` — arbitrary commands work if they don't depend on running services or users
- `apt` config — sources, repos, keys

**Fails in bake mode (silently warned, not fatal):**
- `users` / `groups` — `useradd`/`groupadd` fail without proper PAM/nsswitch in chroot
- `ssh_authorized_keys` — depends on the user existing first
- `ssh_host_keys` — `ssh-keygen` fails without `/dev/urandom` in chroot
- `locale` — `locale-gen` not available
- `ssh_authkey_fingerprints` — depends on user existing
- `growpart` / `resizefs` — no real block devices in chroot
- Any module that depends on running services (systemd units, dbus, etc.)

**Consequence:** If your cloud-init config uses any of the failing modules, `--bake` will fail the build. This is intentional — a "successful" build that silently skipped user setup or SSH keys is worse than a failed build. Use boot-time cloud-init instead (the default, without `--bake`), which runs in a fully booted system where all modules work.

**Rule of thumb:** Use `--bake` for pre-installing packages and writing config files. Use boot-time cloud-init (no `--bake`) for user setup, SSH keys, and service configuration.

**Future option:** Replace the chroot with `systemd-nspawn`, which boots a minimal systemd inside the build environment. This would make all cloud-init modules work in bake mode. Not implemented — revisit if full bake fidelity is needed.

### Machine-ID and host-provided randomness

The initrd generates machine-id from `/proc/sys/kernel/random/uuid`, which is seeded by the hypervisor's vRNG. In a strict TEE threat model, host-provided randomness is untrusted. However:
- The machine-id is on the overlay (not in the verity root), so it doesn't affect measurement
- It's used for systemd journal deduplication, not for security-critical operations
- If stronger guarantees are needed, derive machine-id from attestation-bound material

## Audit Findings to Address

### Must fix
- [ ] QEMU comma-injection: reject paths containing commas before QEMU arg interpolation
- [ ] Launch digest / file hashes: verify at `run` time or label as "(unverified, build-time only)"
- [x] `switch_root`: use util-linux `switch_root` (was available in initrd)
- [x] `CloudInitCleanup::drop` off-by-one: fixed to check `dir.ends_with` not `parent.ends_with`
- [x] Roothash length validation in seal.rs (must be 64/96/128 hex chars)
- [x] Roothash regex in init: normalize to lowercase before validation
- [x] Glob expansion in cmdline parsing: added `set -f` before parse loop

### Should fix
- [x] `safe_path()` was dead code — removed
- [x] `base.rs` now uses `resolve_mkosi()` like `seal.rs`
- [x] Bake cloud-init failures now fail the build (was warn-and-continue)
- [x] `cloud-init clean` now removes all state (was `--logs` only)
- [ ] Add tests for injection rejection (comma paths, deny_unknown_fields, validate_memory)
- [ ] E2E test references non-existent `--service-port` flag
- [ ] `insmod` → `modprobe` to avoid fragile ordering dependency
- [ ] Add `overflow-checks = true` to release profile
- [ ] Remove unreferenced `util-linux-extra` .deb from repo root

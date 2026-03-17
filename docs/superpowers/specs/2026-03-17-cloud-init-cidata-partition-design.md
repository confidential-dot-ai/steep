# Design: cloud-init cidata partition via static mkosi config

**Date:** 2026-03-17

## Summary

Replace the dynamic `MkosiConfig`-based project partition build in `steep cloud-init` with a static mkosi config directory (`mkosi/cidata/`) that builds a minimal vfat cidata partition. The user's cloud-init directory is passed at runtime via `--extra-trees`. Remove `--service-port` from the CLI; users are responsible for opening ports in their `user-data`. Install `cloud-init` in the base image so it can discover and apply the cidata partition at boot.

## Motivation

The current `cloud-init` subcommand uses mkosi to build a full Ubuntu project partition — an unnecessary overhead for what is essentially a two-file data partition. It also codifies firewall config (`--service-port`) at the `steep` level, when that policy belongs in the user's cloud-init `user-data`. The base image does not currently include `cloud-init`, so the cidata partition is never actually applied at boot.

## Changes

### `mkosi/base/mkosi.conf` — install cloud-init in base image

Add a `[Content]` section:

```ini
[Content]
Packages=cloud-init
```

The existing default-deny nftables rules in `mkosi.postinst.d/00-nftables.sh` remain unchanged. Cloud-init applies user-data rules on top of them at first boot.

### `mkosi/cidata/mkosi.conf` — new static config directory

New checked-in file. No distribution, no packages — just a vfat filesystem:

```ini
[Output]
Format=vfat
Label=cidata
Output=cidata.raw
```

### `src/commands/cloud_init.rs` — invoke mkosi directly

Replace the `MkosiConfig::cloud_init()` + nftables postinst approach with a direct mkosi invocation, following the same pattern as `commands/base.rs`:

```
mkosi --directory mkosi/cidata --output-dir <tempdir> --extra-trees <args.dir> build
```

- `<args.dir>` is the user's cloud-init directory; mkosi copies its contents into the image root
- Output is `cidata.raw` in the temp dir, copied to the pipeline as the project partition
- Remove the `nftables::service_rules()` call and `--service-port` usage

### `src/lib.rs` — remove `--service-port`, update help

- Remove `service_port: u16` from `CloudInitArgs`
- Update the `cloud-init` subcommand doc string:
  > "Build a CVM image with cloud-init configuration. The cloud-init user-data must configure any required firewall rules (e.g. opening a service port with nftables)."

### `src/mkosi/config.rs` — remove CloudInit profile

- Remove `MkosiProfile::CloudInit`
- Remove `MkosiConfig::cloud_init()`
- Remove `cloud_init_dir: Option<PathBuf>` field from `MkosiConfig`

### `src/compose/disk.rs` — shrink cidata partition size

Reduce `SizeMinBytes` in `project_partition_conf` from `512M` to `8M`. The cidata partition holds only a few YAML files.

### `src/nftables.rs` — no change

`service_rules()` remains; it is still used by `steep container`.

## Data flow

```
steep cloud-init <dir> ...
  │
  ├─ mkosi --directory mkosi/cidata --extra-trees <dir> → cidata.raw
  │     (vfat, label=cidata, contains meta-data + user-data)
  │
  └─ pipeline:
       compose (base.raw + cidata.raw → disk.raw via repart)
       → ukify → igvm-tools → qemu-img → manifest
```

At boot, the base system (Ubuntu + cloud-init) discovers the cidata partition by filesystem label and applies `user-data`.

## What is NOT changed

- `steep container` — still uses `MkosiConfig::cloud_init()` ... wait, no. Container uses `MkosiConfig::container()`, which is unaffected.
- `steep base` — unaffected.
- `nftables.rs` — `service_rules()` kept for container command.
- The shared pipeline stages (ukify, igvm-tools, repart, qemu-img, manifest) — unaffected.

## Testing

- Remove tests for `MkosiConfig::cloud_init()` (or the cloud-init profile) from `tests/mkosi_config.rs`.
- Verify `steep cloud-init` no longer requires or accepts `--service-port`.
- Verify the cidata partition produced by mkosi contains the expected files at the image root.
- Verify the base image build includes the `cloud-init` package.

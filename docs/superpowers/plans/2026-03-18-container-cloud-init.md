# Container Cloud-Init Delegation Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace mkosi-based container partition building with cloud-init directory generation that delegates to the existing `cloud-init` subcommand code.

**Architecture:** The `container` subcommand generates a cloud-init directory (user-data + meta-data) that installs podman, pulls the container image at boot, sets up nftables rules, and configures a quadlet unit. It then constructs a `CloudInitArgs` and calls `commands::cloud_init::run()` directly. No more mkosi, podman, or OCI archive bundling on the build host.

**Tech Stack:** Rust, cloud-init (cloud-config YAML), podman quadlet, nftables

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `src/container.rs` | Modify | Remove `pull`, `save`, `podman_postinst`; add `user_data` and `meta_data` generators |
| `src/commands/container.rs` | Rewrite | Generate cloud-init dir, delegate to `cloud_init::run()` |
| `tests/container.rs` | Rewrite | Test new `user_data`, `meta_data`, keep `quadlet` tests |
| `tests/mkosi_config.rs` | Modify | Remove container-specific tests, migrate generic method tests to use `repart()` |
| `src/mkosi/config.rs` | Modify | Remove `Container` profile, `container()` constructor, and unused methods/fields |
| `examples/container/run.sh` | Modify | Remove local podman build, use public image (image now pulled at boot) |
| `examples/container/Dockerfile` | Delete | No longer needed — not building a local image |
| `examples/container/index.html` | Delete | No longer needed |
| `examples/container/Caddyfile` | Delete | No longer needed |

---

### Task 1: Add cloud-init user-data and meta-data generators to `container.rs`

**Files:**
- Modify: `src/container.rs`
- Test: `tests/container.rs`

- [ ] **Step 1: Write failing tests for `user_data()` and `meta_data()`**

Replace the entire `tests/container.rs` with:

```rust
use steep::container;

// --- quadlet tests (unchanged) ---

#[test]
fn test_quadlet_contains_image() {
    let quadlet = container::quadlet("ghcr.io/org/app:latest", 8080);
    assert!(quadlet.contains("Image=ghcr.io/org/app:latest"));
}

#[test]
fn test_quadlet_contains_publish_port() {
    let quadlet = container::quadlet("ghcr.io/org/app:latest", 8080);
    assert!(quadlet.contains("PublishPort=8080:8080"));
}

#[test]
fn test_quadlet_has_restart_always() {
    let quadlet = container::quadlet("ghcr.io/org/app:latest", 443);
    assert!(quadlet.contains("Restart=always"));
}

#[test]
fn test_quadlet_has_install_section() {
    let quadlet = container::quadlet("ghcr.io/org/app:latest", 443);
    assert!(quadlet.contains("[Install]"));
    assert!(quadlet.contains("WantedBy=multi-user.target default.target"));
}

// --- user_data tests ---

#[test]
fn test_user_data_starts_with_cloud_config() {
    let ud = container::user_data("ghcr.io/org/app:latest", 8080);
    assert!(ud.starts_with("#cloud-config\n"));
}

#[test]
fn test_user_data_installs_podman() {
    let ud = container::user_data("ghcr.io/org/app:latest", 8080);
    assert!(ud.contains("- podman"));
}

#[test]
fn test_user_data_installs_nftables() {
    let ud = container::user_data("ghcr.io/org/app:latest", 8080);
    assert!(ud.contains("- nftables"));
}

#[test]
fn test_user_data_pulls_container() {
    let ud = container::user_data("ghcr.io/org/app:latest", 8080);
    assert!(ud.contains("podman pull ghcr.io/org/app:latest"));
}

#[test]
fn test_user_data_writes_nftables_rules() {
    let ud = container::user_data("ghcr.io/org/app:latest", 8080);
    assert!(ud.contains("tcp dport 8080 accept"));
}

#[test]
fn test_user_data_writes_quadlet() {
    let ud = container::user_data("ghcr.io/org/app:latest", 8080);
    assert!(ud.contains("/etc/containers/systemd/app.container"));
    assert!(ud.contains("Image=ghcr.io/org/app:latest"));
}

#[test]
fn test_user_data_applies_nftables_before_pull() {
    let ud = container::user_data("ghcr.io/org/app:latest", 8080);
    let nft_pos = ud.find("nft -f").unwrap();
    let pull_pos = ud.find("podman pull").unwrap();
    assert!(nft_pos < pull_pos, "nftables must be applied before podman pull");
}

// --- meta_data tests ---

#[test]
fn test_meta_data_has_instance_id() {
    let md = container::meta_data();
    assert!(md.contains("instance-id:"));
}

#[test]
fn test_meta_data_has_hostname() {
    let md = container::meta_data();
    assert!(md.contains("local-hostname:"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test container`
Expected: compilation errors (functions don't exist yet)

- [ ] **Step 3: Implement `user_data()` and `meta_data()` in `container.rs`**

Replace the entire `src/container.rs` with:

```rust
use crate::nftables;

/// Generate a podman quadlet .container unit file.
pub fn quadlet(url: &str, service_port: u16) -> String {
    format!(
        "[Container]\n\
         Image={url}\n\
         PublishPort={service_port}:{service_port}\n\
         \n\
         [Service]\n\
         Restart=always\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target default.target\n"
    )
}

/// Generate cloud-init user-data that sets up the container workload.
///
/// Installs podman and nftables, writes firewall rules and a quadlet unit,
/// then pulls the container image and starts the service.
pub fn user_data(url: &str, service_port: u16) -> String {
    let nft_rules = nftables::service_rules(service_port);
    let quadlet_content = quadlet(url, service_port);

    let mut s = String::new();
    s.push_str("#cloud-config\n");
    s.push_str("packages:\n");
    s.push_str("  - podman\n");
    s.push_str("  - nftables\n");
    s.push_str("\n");
    s.push_str("write_files:\n");
    s.push_str("  - path: /etc/nftables.conf\n");
    s.push_str("    content: |\n");
    for line in nft_rules.lines() {
        s.push_str(&format!("      {line}\n"));
    }
    s.push_str("  - path: /etc/containers/systemd/app.container\n");
    s.push_str("    content: |\n");
    for line in quadlet_content.lines() {
        s.push_str(&format!("      {line}\n"));
    }
    s.push_str("\n");
    s.push_str("runcmd:\n");
    s.push_str("  - nft -f /etc/nftables.conf\n");
    s.push_str(&format!("  - podman pull {url}\n"));
    s.push_str("  - systemctl daemon-reload\n");
    s.push_str("  - systemctl start app\n");
    s
}

/// Generate cloud-init meta-data for a container workload.
pub fn meta_data() -> String {
    "instance-id: steep-container\nlocal-hostname: steep\n".to_string()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test container`
Expected: all pass

- [ ] **Step 5: Commit**

```bash
git add src/container.rs tests/container.rs
git commit -m "feat: add cloud-init user-data/meta-data generators to container module"
```

---

### Task 2: Rewrite `commands/container.rs` to delegate to cloud-init

**Files:**
- Rewrite: `src/commands/container.rs`

- [ ] **Step 1: Rewrite the container command**

Replace the entire `commands/container.rs` with:

```rust
use crate::container as container_helpers;
use crate::commands::cloud_init;
use crate::{CloudInitArgs, ContainerArgs};

pub fn run(args: &ContainerArgs) -> anyhow::Result<()> {
    tracing::info!(url = %args.url, "building container CVM image");

    // Generate cloud-init directory
    let cloud_init_dir = tempfile::tempdir()?;
    fs_err::write(
        cloud_init_dir.path().join("user-data"),
        container_helpers::user_data(&args.url, args.service_port),
    )?;
    fs_err::write(
        cloud_init_dir.path().join("meta-data"),
        container_helpers::meta_data(),
    )?;
    tracing::info!(dir = %cloud_init_dir.path().display(), "generated cloud-init directory");

    // Delegate to cloud-init command
    cloud_init::run(&CloudInitArgs {
        dir: cloud_init_dir.path().to_path_buf(),
        kernel: args.kernel.clone(),
        initrd: args.initrd.clone(),
        firmware: args.firmware.clone(),
        base_image: args.base_image.clone(),
        memory: args.memory.clone(),
        smp: args.smp,
        format: args.format.clone(),
        output: args.output.clone(),
    })
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: success

- [ ] **Step 3: Run all tests**

Run: `cargo test`
Expected: all pass (CLI tests still work since arg parsing is unchanged)

- [ ] **Step 4: Commit**

```bash
git add src/commands/container.rs
git commit -m "feat: rewrite container command to delegate to cloud-init"
```

---

### Task 3: Remove dead code

After removing the container mkosi path, the following become dead code:
- `MkosiProfile::Container` — no longer constructed
- `MkosiConfig::container()` — no callers
- `add_postinst_script()` — only the container path called it
- `write_postinst_scripts()` — called from `invoke()` but never populated by `repart` path (no-op)
- `postinst_scripts` field — never populated outside removed code
- `add_extra_file()` — only the container path called it
- `write_extra_files()` — called from `invoke()` but never populated by `repart` path (no-op)
- `extra_files` field — never populated outside removed code

The compiler will flag `add_postinst_script`, `add_extra_file`, `MkosiProfile::Container`, and `MkosiConfig::container()` as dead code. The `write_*` methods and fields won't be flagged (they're called from `invoke()`), but they're effectively dead since nothing populates the vectors. Remove them all.

**Files:**
- Modify: `src/mkosi/config.rs`
- Modify: `tests/mkosi_config.rs`

- [ ] **Step 1: Remove dead code from `MkosiConfig`**

In `src/mkosi/config.rs`:
- Remove `Container` variant from `MkosiProfile`
- Remove `container()` constructor
- Remove `postinst_scripts` field and `add_postinst_script()`, `write_postinst_scripts()` methods
- Remove `extra_files` field and `add_extra_file()`, `write_extra_files()` methods
- Remove the calls to `write_postinst_scripts()` and `write_extra_files()` from `invoke()`

The resulting `MkosiConfig` should be:

```rust
use std::path::PathBuf;

/// mkosi build profile.
#[derive(Debug, PartialEq)]
pub enum MkosiProfile {
    Repart,
}

/// Represents a mkosi configuration to be written as an INI file.
pub struct MkosiConfig {
    pub profile: MkosiProfile,
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl MkosiConfig {
    /// Create a mkosi config for disk composition via repart.
    pub fn repart(definitions_dir: PathBuf, output: PathBuf) -> Self {
        let mut config = Self {
            profile: MkosiProfile::Repart,
            sections: Vec::new(),
        };
        config.sections.push((
            "Distribution".to_string(),
            vec![("Distribution".to_string(), "ubuntu".to_string())],
        ));
        config.sections.push((
            "Content".to_string(),
            vec![("RepartDirectories".to_string(), definitions_dir.display().to_string())],
        ));
        config.sections.push((
            "Output".to_string(),
            vec![
                ("Format".to_string(), "disk".to_string()),
                ("Output".to_string(), output.display().to_string()),
            ],
        ));
        config
    }

    /// Serialize to mkosi INI format.
    pub fn to_ini(&self) -> String {
        let mut output = String::new();
        for (section, entries) in &self.sections {
            output.push_str(&format!("[{}]\n", section));
            for (key, value) in entries {
                output.push_str(&format!("{}={}\n", key, value));
            }
            output.push('\n');
        }
        output
    }

    /// Write the config to a file.
    pub fn write_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        fs_err::write(path, self.to_ini())?;
        Ok(())
    }

    /// Build the mkosi command-line arguments.
    pub fn to_mkosi_args(&self, work_dir: &std::path::Path) -> Vec<String> {
        vec![
            "--directory".to_string(),
            work_dir.display().to_string(),
            "--output-dir".to_string(),
            work_dir.display().to_string(),
            "build".to_string(),
        ]
    }

    /// Invoke mkosi with the generated config.
    pub fn invoke(&self, work_dir: &std::path::Path) -> anyhow::Result<()> {
        let config_path = work_dir.join("mkosi.conf");
        self.write_to(&config_path)?;
        crate::tools::require("mkosi")?;
        let args = self.to_mkosi_args(work_dir);
        tracing::info!(config = %config_path.display(), "invoking mkosi");
        crate::tools::run_command_streaming("mkosi", &args)?;
        Ok(())
    }
}
```

- [ ] **Step 2: Update mkosi config tests**

In `tests/mkosi_config.rs`, remove container-specific tests and keep only repart:

```rust
use std::path::PathBuf;
use steep::mkosi::config::{MkosiConfig, MkosiProfile};

#[test]
fn test_repart_config() {
    let config = MkosiConfig::repart(
        PathBuf::from("/path/to/definitions"),
        PathBuf::from("/path/to/output.raw"),
    );
    assert_eq!(config.profile, MkosiProfile::Repart);
    let ini = config.to_ini();
    assert!(ini.contains("[Output]"));
}
```

- [ ] **Step 3: Verify compilation and tests**

Run: `cargo test`
Expected: all pass, no dead code warnings

- [ ] **Step 4: Commit**

```bash
git add src/mkosi/config.rs tests/mkosi_config.rs
git commit -m "remove: dead container-related mkosi code"
```

---

### Task 4: Update container example

The container image is now pulled at boot inside the VM, so the demo can no longer build a local image and reference it by tag. The staleness-checking logic (`.container-image-id`) is also unnecessary since image changes don't require a rebuild — the image is pulled fresh at boot.

**Files:**
- Modify: `examples/container/run.sh`
- Delete: `examples/container/Dockerfile`
- Delete: `examples/container/index.html`
- Delete: `examples/container/Caddyfile`

- [ ] **Step 1: Update the demo script**

Replace `examples/container/run.sh` with:

```bash
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

FORCE=0
for arg in "$@"; do
    [[ "$arg" == "--force" ]] && FORCE=1
done

STEEP="$REPO_ROOT/target/release/steep"
IGVM_PREBUILT="$(cd "$REPO_ROOT/../igvm-tools/examples/prebuilt" && pwd)"
KERNEL="$IGVM_PREBUILT/uki.efi"
FIRMWARE="$IGVM_PREBUILT/OVMF.fd"
BASE_IMAGE="$REPO_ROOT/output/demo/base/base.raw"
OUTPUT="$REPO_ROOT/output/demo/container"
IMAGE="docker.io/library/caddy:latest"
PORT=8081

# Build steep if not already built
if [[ ! -x "$STEEP" ]]; then
    (cd "$REPO_ROOT" && cargo build --release --quiet)
fi

# Remove output dir if --force
if [[ $FORCE -eq 1 ]]; then
    rm -rf "$OUTPUT"
fi

# Build base image if not present
if [[ ! -f "$BASE_IMAGE" ]]; then
    echo "==> Building base image..."
    "$STEEP" base \
        -o "$REPO_ROOT/output/demo/base"
fi

if [[ ! -f "$OUTPUT/manifest.json" ]]; then
    echo "==> Building container CVM image..."
    "$STEEP" container "$IMAGE" \
        --kernel "$KERNEL" \
        --firmware "$FIRMWARE" \
        --base-image "$BASE_IMAGE" \
        --service-port 80 \
        -o "$OUTPUT"
fi

echo ""
echo "==> Container demo ready."
echo "    URL: http://localhost:$PORT"
echo "    (caddy takes ~10-30s to start after the VM boots)"
echo ""

sudo "$STEEP" run --port-forward "${PORT}:80" "$OUTPUT"
```

- [ ] **Step 2: Delete unused example files**

Remove `examples/container/Dockerfile`, `examples/container/index.html`, and `examples/container/Caddyfile`.

- [ ] **Step 3: Commit**

```bash
git rm examples/container/Dockerfile examples/container/index.html examples/container/Caddyfile
git add examples/container/run.sh
git commit -m "update: container example to use public image with cloud-init"
```

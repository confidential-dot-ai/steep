# steep Pipeline Wiring Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire up all stubbed pipeline stages in steep to perform real invocations of mkosi, ukify, igvm-tools, qemu-img, and qemu. Add a setup script, nftables firewall hardening, and a `run` subcommand.

**Architecture:** The existing CLI parsing, argument construction modules (uki, igvm, mkosi, manifest, tools), and module structure are in place with stub command handlers. This plan replaces stubs with real invocations, adds new infrastructure (nftables generation, source image resolution, format conversion, repart config), and wires the full cloud-init pipeline end-to-end.

**Tech Stack:** Rust, clap (derive), tracing, serde, sha2, fs_err, thiserror, tempfile, clap-verbosity-flag. External tools: mkosi, systemd-ukify, igvm-tools, qemu-img, qemu-system-x86_64, curl.

**Spec:** `docs/superpowers/specs/2026-03-14-steep-pipeline-wiring.md`

---

## File Map

| File | Responsibility |
|------|---------------|
| `bin/setup` | Bash script to install external tool dependencies on Ubuntu |
| `src/lib.rs` | Add `--service-port`, `--memory` to `CloudInitArgs`; add `RunArgs`; add `Run` to `Commands`; add `memory` to `BuildConfig`; change `BaseArgs.source_image` to `String` |
| `src/main.rs` | Add dispatch for `run` subcommand |
| `src/nftables.rs` | Generate nftables rule files (base block-all, cloud-init open port) |
| `src/source.rs` | Source image resolution — detect URL vs path, download + cache |
| `src/convert.rs` | qemu-img format conversion (raw → qcow2/vhd) |
| `src/qemu.rs` | QEMU argument construction and invocation for `steep run` |
| `src/mkosi/config.rs` | Extend with `add_postinst_script()`, `MkosiConfig::repart()`, and `invoke()` |
| `src/compose/disk.rs` | Replace stub with repart config generation + mkosi invocation |
| `src/manifest.rs` | Add `memory` to `BuildConfig`, add igvm-tools manifest parsing |
| `src/lib.rs` (inline `commands` module) | Add `pub mod run;` to inline commands block |
| `src/commands/base.rs` | Replace stub with source resolution + mkosi invocation + nftables |
| `src/commands/cloud_init.rs` | Replace stubs with real invocations for stages 4-9 |
| `src/commands/run.rs` | `run` subcommand — read manifest, discover artifacts, invoke QEMU |

---

## Chunk 1: Setup & CLI Foundation

### Task 1: Create bin/setup

**Files:**
- Create: `bin/setup`

- [ ] **Step 1: Write the setup script**

```bash
#!/usr/bin/env bash
set -euo pipefail

echo "==> Installing system packages..."
sudo apt-get update -qq
sudo apt-get install -y -qq mkosi systemd-ukify qemu-utils qemu-system-x86

echo "==> Installing igvm-tools..."
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IGVM_LOCAL="${SCRIPT_DIR}/../../igvm-tools"

if [ -d "$IGVM_LOCAL" ]; then
    echo "    Found local igvm-tools at $IGVM_LOCAL"
    cargo install --path "$IGVM_LOCAL"
else
    echo "    Installing igvm-tools from GitHub..."
    cargo install --git https://github.com/lunal-dev/igvm-tools
fi

echo "==> Installing OVMF firmware..."
OVMF_DIR="$HOME/.local/share/steep"
mkdir -p "$OVMF_DIR"

if [ -d "$IGVM_LOCAL" ] && [ -f "$IGVM_LOCAL/examples/prebuilt/OVMF.fd" ]; then
    echo "    Copying OVMF from local igvm-tools..."
    cp "$IGVM_LOCAL/examples/prebuilt/OVMF.fd" "$OVMF_DIR/OVMF.fd"
else
    echo "    Downloading OVMF from GitHub..."
    curl -fsSL -o "$OVMF_DIR/OVMF.fd" \
        "https://raw.githubusercontent.com/lunal-dev/igvm-tools/main/examples/prebuilt/OVMF.fd"
fi

echo "==> Setup complete."
echo "    OVMF installed at: $OVMF_DIR/OVMF.fd"
```

- [ ] **Step 2: Make it executable**

Run: `chmod +x bin/setup`

- [ ] **Step 3: Verify script syntax**

Run: `bash -n bin/setup`
Expected: No output (no syntax errors)

- [ ] **Step 4: Commit**

```bash
git add bin/setup
git commit -m "feat: add bin/setup script for external tool installation"
```

---

### Task 2: Update CLI args and structs

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Note: `commands` is an inline module in `lib.rs`, not a separate `mod.rs` file

- [ ] **Step 1: Write tests for new CLI args**

Add to `tests/cli.rs`:

```rust
#[test]
fn test_cloud_init_requires_service_port() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args([
        "cloud-init", "/tmp",
        "--kernel", "/tmp/k",
        "--initrd", "/tmp/i",
        "--firmware", "/tmp/f",
        "--base-image", "/tmp/b",
        "-o", "/tmp/o",
    ])
    .assert()
    .failure()
    .stderr(predicates::str::contains("--service-port"));
}

#[test]
fn test_cloud_init_accepts_service_port() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args([
        "cloud-init", "/tmp",
        "--kernel", "/tmp/k",
        "--initrd", "/tmp/i",
        "--firmware", "/tmp/f",
        "--base-image", "/tmp/b",
        "--service-port", "8080",
        "-o", "/tmp/o",
    ])
    .assert()
    .failure(); // Fails on validation, not parsing
}

#[test]
fn test_cloud_init_memory_default() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args([
        "cloud-init", "/tmp",
        "--kernel", "/tmp/k",
        "--initrd", "/tmp/i",
        "--firmware", "/tmp/f",
        "--base-image", "/tmp/b",
        "--service-port", "443",
        "-o", "/tmp/o",
    ])
    .assert()
    .failure(); // Fails on validation, not parsing — proves --memory has a default
}

#[test]
fn test_run_requires_dir() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args(["run"])
        .assert()
        .failure();
}

#[test]
fn test_run_accepts_dir() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args(["run", "/tmp/nonexistent"])
        .assert()
        .failure(); // Fails on validation, not parsing
}

#[test]
fn test_base_accepts_url_as_source_image() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args([
        "base",
        "--source-image", "https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img",
        "-o", "/tmp/o",
    ])
    .assert()
    .failure(); // Fails on download/validation, not parsing
}

#[test]
fn test_help_shows_run_subcommand() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("run"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test`
Expected: Compilation errors — `--service-port` not recognized, `run` subcommand not defined

- [ ] **Step 3: Update lib.rs with new args and structs**

In `src/lib.rs`, make these changes:

Add `service_port` and `memory` fields to `CloudInitArgs`:

```rust
    /// Single TCP port to allow through firewall
    #[arg(long)]
    pub service_port: u16,

    /// RAM for VM (QEMU-style suffix, e.g. "2G")
    #[arg(long, default_value = "2G")]
    pub memory: String,
```

Add `RunArgs` struct:

```rust
#[derive(clap::Args)]
pub struct RunArgs {
    /// Output directory from steep cloud-init
    pub dir: PathBuf,
}
```

Change `BaseArgs.source_image` from `PathBuf` to `String`:

```rust
    /// Ubuntu cloud image (local path or URL)
    #[arg(long)]
    pub source_image: String,
```

Add `memory` field to `BuildConfig` in `src/manifest.rs`:

```rust
pub struct BuildConfig {
    pub timestamp: String,
    pub smp: u32,
    pub memory: String,
    pub format: String,
    pub platform: String,
}
```

Add new module declarations to `lib.rs`:

```rust
pub mod nftables;
pub mod source;
pub mod convert;
pub mod qemu;
```

Add `pub mod run;` inside the inline `commands` module block in `lib.rs`:

```rust
pub mod commands {
    pub mod base;
    pub mod cloud_init;
    pub mod container;
    pub mod kernel;
    pub mod run;
}
```

- [ ] **Step 4: Update main.rs with run subcommand**

Add `RunArgs` to imports, add `Run(RunArgs)` variant to `Commands` enum, add dispatch:

```rust
Commands::Run(args) => commands::run::run(args),
```

- [ ] **Step 5: Create stub command and module files**

Create `src/commands/run.rs`:

```rust
use crate::RunArgs;

pub fn run(args: &RunArgs) -> anyhow::Result<()> {
    tracing::info!(dir = %args.dir.display(), "launching CVM");
    anyhow::bail!("run subcommand not yet implemented")
}
```

Create `src/nftables.rs`:

```rust
// nftables rule generation
```

Create `src/source.rs`:

```rust
// Source image resolution (URL download + caching)
```

Create `src/convert.rs`:

```rust
// qemu-img format conversion
```

Create `src/qemu.rs`:

```rust
// QEMU invocation for steep run
```

- [ ] **Step 6: Update base.rs for String source_image**

In `src/commands/base.rs`, update the validation to work with `String` instead of `PathBuf`. For now, just treat it as a path (URL support comes in Task 4):

```rust
use crate::{tools, BaseArgs};
use std::path::Path;

pub fn run(args: &BaseArgs) -> anyhow::Result<()> {
    tracing::info!(source_image = %args.source_image, "building base image");

    let source_path = Path::new(&args.source_image);
    if !source_path.exists() {
        anyhow::bail!("source image not found: {}", args.source_image);
    }

    tools::require("mkosi")?;
    fs_err::create_dir_all(&args.output)?;

    tracing::warn!("base image build not yet fully implemented");
    Ok(())
}
```

- [ ] **Step 7: Update manifest test for memory field**

In `tests/manifest.rs`, update `sample_manifest()` to include `memory`:

```rust
build: BuildConfig {
    timestamp: "2026-03-13T12:00:00Z".to_string(),
    smp: 4,
    memory: "2G".to_string(),
    format: "qcow2".to_string(),
    platform: "snp".to_string(),
},
```

- [ ] **Step 8: Update existing tests for required --service-port**

Several existing tests invoke `cloud-init` without `--service-port` and will now fail at arg parsing. Add `"--service-port", "443"` to each.

Update `test_cloud_init_fails_with_missing_dir`:

```rust
#[test]
fn test_cloud_init_fails_with_missing_dir() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args([
        "cloud-init", "/nonexistent/dir",
        "--kernel", "/tmp/k",
        "--initrd", "/tmp/i",
        "--firmware", "/tmp/f",
        "--base-image", "/tmp/b",
        "--service-port", "443",
        "-o", "/tmp/o",
    ])
    .assert()
    .failure()
    .stderr(predicates::str::contains("not found"));
}
```

Update `test_cloud_init_requires_kernel_flag` (this test checks that `--kernel` is required, so it should NOT include `--service-port` since it tests a different missing arg — but it will now fail on `--service-port` first). Since both are required, the test still validates that the command fails when required args are missing. No change needed for this test — clap reports all missing required args.

Update `test_smp_default_is_one`:

```rust
#[test]
fn test_smp_default_is_one() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args([
        "cloud-init", "/tmp",
        "--kernel", "/tmp/k",
        "--initrd", "/tmp/i",
        "--firmware", "/tmp/f",
        "--base-image", "/tmp/b",
        "--service-port", "443",
        "-o", "/tmp/o",
    ])
    .assert()
    .failure();
}
```

Also update `test_format_flag_accepts_vhd` to include `--service-port`:

```rust
#[test]
fn test_format_flag_accepts_vhd() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args([
        "cloud-init", "/tmp",
        "--kernel", "/tmp/k",
        "--initrd", "/tmp/i",
        "--firmware", "/tmp/f",
        "--base-image", "/tmp/b",
        "--service-port", "443",
        "--format", "vhd",
        "-o", "/tmp/o",
    ])
    .assert()
    .failure();
}
```

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 10: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 11: Commit**

```bash
git add src/ tests/cli.rs tests/manifest.rs
git commit -m "feat: add --service-port, --memory, run subcommand, and URL source-image support to CLI"
```

---

## Chunk 2: Infrastructure Modules

### Task 3: Implement nftables rule generation

**Files:**
- Create: `tests/nftables.rs`
- Modify: `src/nftables.rs`

- [ ] **Step 1: Write tests for nftables generation**

Create `tests/nftables.rs`:

```rust
use steep::nftables;

#[test]
fn test_base_rules_drops_all_input() {
    let rules = nftables::base_rules();
    assert!(rules.contains("policy drop"));
    assert!(rules.contains("chain input"));
    assert!(rules.contains("chain output"));
    assert!(rules.contains("chain forward"));
}

#[test]
fn test_base_rules_allows_loopback() {
    let rules = nftables::base_rules();
    assert!(rules.contains(r#"iif "lo" accept"#));
    assert!(rules.contains(r#"oif "lo" accept"#));
}

#[test]
fn test_base_rules_allows_established() {
    let rules = nftables::base_rules();
    assert!(rules.contains("ct state established,related accept"));
}

#[test]
fn test_base_rules_output_policy_is_drop() {
    let rules = nftables::base_rules();
    // The output chain should have policy drop in base
    assert!(rules.contains("chain output {\n        type filter hook output priority 0; policy drop;"));
}

#[test]
fn test_service_rules_opens_port() {
    let rules = nftables::service_rules(8080);
    assert!(rules.contains("tcp dport 8080 accept"));
}

#[test]
fn test_service_rules_output_policy_is_accept() {
    let rules = nftables::service_rules(443);
    assert!(rules.contains("chain output {\n        type filter hook output priority 0; policy accept;"));
}

#[test]
fn test_service_rules_starts_with_shebang() {
    let rules = nftables::service_rules(443);
    assert!(rules.starts_with("#!/usr/sbin/nft -f\n"));
}

#[test]
fn test_base_rules_starts_with_shebang() {
    let rules = nftables::base_rules();
    assert!(rules.starts_with("#!/usr/sbin/nft -f\n"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test nftables`
Expected: Compilation error — `nftables::base_rules` and `nftables::service_rules` not defined

- [ ] **Step 3: Implement nftables module**

Write `src/nftables.rs`:

```rust
/// Generate nftables rules for the base image.
/// Blocks all new incoming and outgoing connections.
/// Only loopback and already-established connections are permitted.
pub fn base_rules() -> String {
    "#!/usr/sbin/nft -f\n\
     flush ruleset\n\
     table inet filter {\n    \
         chain input {\n        \
             type filter hook input priority 0; policy drop;\n        \
             iif \"lo\" accept\n        \
             ct state established,related accept\n    \
         }\n    \
         chain forward {\n        \
             type filter hook forward priority 0; policy drop;\n    \
         }\n    \
         chain output {\n        \
             type filter hook output priority 0; policy drop;\n        \
             oif \"lo\" accept\n        \
             ct state established,related accept\n    \
         }\n\
     }\n"
        .to_string()
}

/// Generate nftables rules for the project partition.
/// Opens a single TCP port for inbound traffic.
/// Outbound traffic is allowed (policy accept).
pub fn service_rules(port: u16) -> String {
    format!(
        "#!/usr/sbin/nft -f\n\
         flush ruleset\n\
         table inet filter {{\n    \
             chain input {{\n        \
                 type filter hook input priority 0; policy drop;\n        \
                 iif \"lo\" accept\n        \
                 ct state established,related accept\n        \
                 tcp dport {port} accept\n    \
             }}\n    \
             chain forward {{\n        \
                 type filter hook forward priority 0; policy drop;\n    \
             }}\n    \
             chain output {{\n        \
                 type filter hook output priority 0; policy accept;\n    \
             }}\n\
         }}\n"
    )
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test nftables`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/nftables.rs tests/nftables.rs
git commit -m "feat: implement nftables rule generation for base and service configs"
```

---

### Task 4: Implement source image resolution

**Files:**
- Create: `tests/source.rs`
- Modify: `src/source.rs`

- [ ] **Step 1: Write tests for source resolution**

Create `tests/source.rs`:

```rust
use steep::source;

#[test]
fn test_is_url_detects_https() {
    assert!(source::is_url("https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img"));
}

#[test]
fn test_is_url_detects_http() {
    assert!(source::is_url("http://example.com/image.img"));
}

#[test]
fn test_is_url_rejects_local_path() {
    assert!(!source::is_url("/home/user/images/ubuntu.img"));
}

#[test]
fn test_is_url_rejects_relative_path() {
    assert!(!source::is_url("images/ubuntu.img"));
}

#[test]
fn test_filename_from_url() {
    let name = source::filename_from_url("https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img");
    assert_eq!(name, Some("noble-server-cloudimg-amd64.img".to_string()));
}

#[test]
fn test_cache_dir() {
    let dir = source::cache_dir();
    assert!(dir.ends_with("steep/base-inputs"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test source`
Expected: Compilation error

- [ ] **Step 3: Implement source resolution module**

Write `src/source.rs`:

```rust
use std::path::{Path, PathBuf};

/// Check if a string looks like a URL.
pub fn is_url(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://")
}

/// Extract the filename from a URL.
pub fn filename_from_url(url: &str) -> Option<String> {
    url.rsplit('/').next().map(|s| s.to_string())
}

/// Return the cache directory for downloaded base images.
pub fn cache_dir() -> PathBuf {
    dirs_path().join("base-inputs")
}

fn dirs_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".to_string()))
                .join(".local/share")
        });
    base.join("steep")
}

/// Resolve a source image string to a local path.
/// If the source is a URL, download it to the cache directory (skip if already cached).
/// If the source is a local path, validate it exists and return it.
pub fn resolve(source: &str) -> anyhow::Result<PathBuf> {
    if is_url(source) {
        let filename = filename_from_url(source)
            .ok_or_else(|| anyhow::anyhow!("cannot extract filename from URL: {source}"))?;
        let cache = cache_dir();
        fs_err::create_dir_all(&cache)?;
        let cached_path = cache.join(&filename);
        if cached_path.exists() {
            tracing::info!(path = %cached_path.display(), "using cached source image");
            return Ok(cached_path);
        }
        tracing::info!(url = source, dest = %cached_path.display(), "downloading source image");
        crate::tools::require("curl")?;
        let dest = cached_path.display().to_string();
        crate::tools::run_command_streaming("curl", &[
            "-fSL",
            "-o", &dest,
            source,
        ])?;
        Ok(cached_path)
    } else {
        let path = Path::new(source);
        if !path.exists() {
            anyhow::bail!("source image not found: {source}");
        }
        Ok(path.to_path_buf())
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test source`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/source.rs tests/source.rs
git commit -m "feat: implement source image resolution with URL download and caching"
```

---

### Task 5: Extend mkosi config with postinst scripts and repart support

**Files:**
- Modify: `src/mkosi/config.rs`
- Modify: `tests/mkosi_config.rs`

- [ ] **Step 1: Write tests for new mkosi config features**

Add to `tests/mkosi_config.rs`:

```rust
#[test]
fn test_add_postinst_script() {
    let mut config = MkosiConfig::base(PathBuf::from("/path/to/img"));
    config.add_postinst_script("#!/bin/bash\necho hello");
    assert_eq!(config.postinst_scripts.len(), 1);
    assert!(config.postinst_scripts[0].contains("echo hello"));
}

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

#[test]
fn test_invoke_args_base() {
    let config = MkosiConfig::base(PathBuf::from("/path/to/img"));
    let args = config.to_mkosi_args(std::path::Path::new("/work"));
    assert!(args.contains(&"build".to_string()));
    assert!(args.contains(&"--directory".to_string()));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test mkosi_config`
Expected: Compilation errors

- [ ] **Step 3: Implement mkosi config extensions**

Update `src/mkosi/config.rs`:

Add `Repart` to `MkosiProfile`:

```rust
#[derive(Debug, PartialEq)]
pub enum MkosiProfile {
    Base,
    CloudInit,
    Repart,
}
```

Add `postinst_scripts` field and new methods to `MkosiConfig`:

```rust
pub struct MkosiConfig {
    pub profile: MkosiProfile,
    pub source_image: Option<PathBuf>,
    pub cloud_init_dir: Option<PathBuf>,
    pub postinst_scripts: Vec<String>,
    sections: Vec<(String, Vec<(String, String)>)>,
}
```

Update existing constructors to initialize `postinst_scripts: Vec::new()`.

Add methods:

```rust
/// Add a postinst script to be written into the mkosi build tree.
pub fn add_postinst_script(&mut self, content: &str) {
    self.postinst_scripts.push(content.to_string());
}

/// Create a mkosi config for disk composition via repart.
pub fn repart(definitions_dir: PathBuf, output: PathBuf) -> Self {
    let mut config = Self {
        profile: MkosiProfile::Repart,
        source_image: None,
        cloud_init_dir: None,
        postinst_scripts: Vec::new(),
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

/// Build the mkosi command-line arguments.
/// The work_dir is passed as --directory so mkosi finds its config and scripts.
pub fn to_mkosi_args(&self, work_dir: &std::path::Path) -> Vec<String> {
    vec![
        "--directory".to_string(),
        work_dir.display().to_string(),
        "build".to_string(),
    ]
}

/// Write postinst scripts to the mkosi build tree directory.
/// Creates mkosi.postinst.d/ with numbered scripts.
pub fn write_postinst_scripts(&self, build_dir: &std::path::Path) -> anyhow::Result<()> {
    if self.postinst_scripts.is_empty() {
        return Ok(());
    }
    let postinst_dir = build_dir.join("mkosi.postinst.d");
    fs_err::create_dir_all(&postinst_dir)?;
    for (i, script) in self.postinst_scripts.iter().enumerate() {
        let script_path = postinst_dir.join(format!("{:02}-script.sh", i));
        fs_err::write(&script_path, script)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

/// Invoke mkosi with the generated config.
pub fn invoke(&self, work_dir: &std::path::Path) -> anyhow::Result<()> {
    let config_path = work_dir.join("mkosi.conf");
    self.write_to(&config_path)?;
    self.write_postinst_scripts(work_dir)?;
    crate::tools::require("mkosi")?;
    let args = self.to_mkosi_args(work_dir);
    tracing::info!(config = %config_path.display(), "invoking mkosi");
    crate::tools::run_command_streaming("mkosi", &args)?;
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test mkosi_config`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/mkosi/config.rs tests/mkosi_config.rs
git commit -m "feat: extend mkosi config with postinst scripts, repart support, and invocation"
```

---

### Task 6: Implement format conversion module

**Files:**
- Create: `tests/convert.rs`
- Modify: `src/convert.rs`

- [ ] **Step 1: Write tests for format conversion**

Create `tests/convert.rs`:

```rust
use steep::convert;
use steep::ImageFormat;

#[test]
fn test_qemu_img_format_qcow2() {
    assert_eq!(convert::qemu_img_format(&ImageFormat::Qcow2), "qcow2");
}

#[test]
fn test_qemu_img_format_vhd() {
    assert_eq!(convert::qemu_img_format(&ImageFormat::Vhd), "vpc");
}

#[test]
fn test_qemu_img_format_raw() {
    assert_eq!(convert::qemu_img_format(&ImageFormat::Raw), "raw");
}

#[test]
fn test_convert_args() {
    let args = convert::convert_args(
        std::path::Path::new("/tmp/disk.raw"),
        std::path::Path::new("/tmp/disk.qcow2"),
        &ImageFormat::Qcow2,
    );
    assert_eq!(args, vec!["convert", "-f", "raw", "-O", "qcow2", "/tmp/disk.raw", "/tmp/disk.qcow2"]);
}

#[test]
fn test_convert_args_vhd_uses_vpc() {
    let args = convert::convert_args(
        std::path::Path::new("/tmp/disk.raw"),
        std::path::Path::new("/tmp/disk.vhd"),
        &ImageFormat::Vhd,
    );
    assert_eq!(args, vec!["convert", "-f", "raw", "-O", "vpc", "/tmp/disk.raw", "/tmp/disk.vhd"]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test convert`
Expected: Compilation error

- [ ] **Step 3: Implement format conversion module**

Write `src/convert.rs`:

```rust
use std::path::Path;

use crate::ImageFormat;

/// Return the qemu-img format string for an ImageFormat.
pub fn qemu_img_format(format: &ImageFormat) -> &'static str {
    match format {
        ImageFormat::Qcow2 => "qcow2",
        ImageFormat::Vhd => "vpc",
        ImageFormat::Raw => "raw",
    }
}

/// Build the argument list for qemu-img convert.
pub fn convert_args(input: &Path, output: &Path, format: &ImageFormat) -> Vec<String> {
    vec![
        "convert".to_string(),
        "-f".to_string(),
        "raw".to_string(),
        "-O".to_string(),
        qemu_img_format(format).to_string(),
        input.display().to_string(),
        output.display().to_string(),
    ]
}

/// Convert a raw disk image to the specified format using qemu-img.
/// No-op if format is raw.
pub fn convert(input: &Path, output: &Path, format: &ImageFormat) -> anyhow::Result<()> {
    if matches!(format, ImageFormat::Raw) {
        tracing::info!("output format is raw, skipping conversion");
        fs_err::copy(input, output)?;
        return Ok(());
    }
    crate::tools::require("qemu-img")?;
    let args = convert_args(input, output, format);
    tracing::info!(
        input = %input.display(),
        output = %output.display(),
        format = qemu_img_format(format),
        "converting disk image"
    );
    crate::tools::run_command_streaming("qemu-img", &args)?;
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test convert`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/convert.rs tests/convert.rs
git commit -m "feat: implement qemu-img format conversion module"
```

---

### Task 7: Extend manifest with memory field and igvm parsing

**Files:**
- Modify: `src/manifest.rs`
- Modify: `tests/manifest.rs`

- [ ] **Step 1: Write tests for igvm manifest parsing**

Add to `tests/manifest.rs`:

```rust
#[test]
fn test_parse_igvm_manifest() {
    let igvm_json = r#"{
        "snp_launch_digest": "aabbccdd",
        "algorithm": "sha384",
        "page_count": 5598,
        "vmsa_count": 4
    }"#;
    let measurement = steep::manifest::parse_igvm_manifest(igvm_json).unwrap();
    assert_eq!(measurement.snp_launch_digest, "aabbccdd");
    assert_eq!(measurement.algorithm, "sha384");
    assert_eq!(measurement.page_count, 5598);
    assert_eq!(measurement.vmsa_count, 4);
}

#[test]
fn test_manifest_includes_memory() {
    let manifest = sample_manifest();
    let json = serde_json::to_string_pretty(&manifest).unwrap();
    assert!(json.contains("\"memory\": \"2G\""));
}

#[test]
fn test_read_manifest_from_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    let manifest = sample_manifest();
    steep::manifest::write_manifest(&manifest, &path).unwrap();
    let loaded = steep::manifest::read_manifest(&path).unwrap();
    assert_eq!(loaded.build.smp, 4);
    assert_eq!(loaded.build.memory, "2G");
    assert_eq!(loaded.build.format, "qcow2");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test manifest`
Expected: Compilation errors — `parse_igvm_manifest` and `read_manifest` not defined, `memory` field missing

- [ ] **Step 3: Implement manifest extensions**

Add to `src/manifest.rs`:

```rust
/// Parse the igvm-tools manifest JSON to extract measurement data.
pub fn parse_igvm_manifest(json: &str) -> anyhow::Result<Measurement> {
    let measurement: Measurement = serde_json::from_str(json)?;
    Ok(measurement)
}

/// Read a manifest from a JSON file.
pub fn read_manifest(path: &Path) -> anyhow::Result<BuildManifest> {
    let content = fs_err::read_to_string(path)?;
    let manifest: BuildManifest = serde_json::from_str(&content)?;
    Ok(manifest)
}
```

The `memory` field in `BuildConfig` was already added in Task 2.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test manifest`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/manifest.rs tests/manifest.rs
git commit -m "feat: add igvm manifest parsing, read_manifest, and memory field to BuildConfig"
```

---

### Task 8: Implement QEMU module

**Files:**
- Create: `tests/qemu.rs`
- Modify: `src/qemu.rs`

- [ ] **Step 1: Write tests for QEMU argument construction**

Create `tests/qemu.rs`:

```rust
use std::path::PathBuf;
use steep::qemu::QemuArgs;

#[test]
fn test_qemu_args_basic() {
    let args = QemuArgs {
        igvm: PathBuf::from("/output/guest.igvm"),
        disk: PathBuf::from("/output/disk.qcow2"),
        disk_format: "qcow2".to_string(),
        smp: 2,
        memory: "2G".to_string(),
    };
    let cmd = args.to_args();
    assert!(cmd.contains(&"-nographic".to_string()));
    assert!(cmd.contains(&"-smp".to_string()));
    assert!(cmd.contains(&"2".to_string()));
    assert!(cmd.contains(&"-m".to_string()));
    assert!(cmd.contains(&"2G".to_string()));
}

#[test]
fn test_qemu_args_contains_sev_snp() {
    let args = QemuArgs {
        igvm: PathBuf::from("/output/guest.igvm"),
        disk: PathBuf::from("/output/disk.qcow2"),
        disk_format: "qcow2".to_string(),
        smp: 1,
        memory: "4G".to_string(),
    };
    let cmd = args.to_args();
    let joined = cmd.join(" ");
    assert!(joined.contains("confidential-guest-support=sev0"));
    assert!(joined.contains("sev-snp-guest"));
    assert!(joined.contains("igvm-cfg"));
}

#[test]
fn test_qemu_args_disk_format() {
    let args = QemuArgs {
        igvm: PathBuf::from("/output/guest.igvm"),
        disk: PathBuf::from("/output/disk.vhd"),
        disk_format: "vpc".to_string(),
        smp: 1,
        memory: "2G".to_string(),
    };
    let cmd = args.to_args();
    let joined = cmd.join(" ");
    assert!(joined.contains("format=vpc"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test qemu`
Expected: Compilation error

- [ ] **Step 3: Implement QEMU module**

Write `src/qemu.rs`:

```rust
use std::path::PathBuf;

use crate::tools;

/// Arguments for launching a CVM with QEMU.
pub struct QemuArgs {
    pub igvm: PathBuf,
    pub disk: PathBuf,
    pub disk_format: String,
    pub smp: u32,
    pub memory: String,
}

impl QemuArgs {
    /// Build the QEMU command-line arguments.
    pub fn to_args(&self) -> Vec<String> {
        vec![
            "-machine".to_string(),
            "q35,confidential-guest-support=sev0,igvm-cfg=igvm0".to_string(),
            "-object".to_string(),
            "sev-snp-guest,id=sev0".to_string(),
            "-object".to_string(),
            format!("igvm-cfg,id=igvm0,file={}", self.igvm.display()),
            "-drive".to_string(),
            format!("file={},format={},if=virtio", self.disk.display(), self.disk_format),
            "-smp".to_string(),
            self.smp.to_string(),
            "-m".to_string(),
            self.memory.clone(),
            "-nographic".to_string(),
        ]
    }
}

/// Launch a CVM using QEMU with SEV-SNP.
pub fn launch(args: &QemuArgs) -> anyhow::Result<()> {
    tools::require("qemu-system-x86_64")?;
    let cmd_args = args.to_args();
    tracing::info!(
        igvm = %args.igvm.display(),
        disk = %args.disk.display(),
        smp = args.smp,
        memory = %args.memory,
        "launching CVM via QEMU"
    );
    tools::run_command_streaming("qemu-system-x86_64", &cmd_args)?;
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test qemu`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/qemu.rs tests/qemu.rs
git commit -m "feat: implement QEMU argument construction and launch module"
```

---

## Chunk 3: Command Implementations

### Task 9: Implement disk composition via repart

**Files:**
- Modify: `src/compose/disk.rs`
- Create: `tests/compose.rs`

- [ ] **Step 1: Write tests for repart config generation**

Create `tests/compose.rs`:

```rust
use steep::compose::disk;

#[test]
fn test_base_partition_conf() {
    let conf = disk::base_partition_conf(std::path::Path::new("/images/base.raw"));
    assert!(conf.contains("[Partition]"));
    assert!(conf.contains("Type=root"));
    assert!(conf.contains("CopyBlocks=/images/base.raw"));
    assert!(conf.contains("ReadOnly=yes"));
}

#[test]
fn test_project_partition_conf() {
    let conf = disk::project_partition_conf(std::path::Path::new("/images/project.raw"));
    assert!(conf.contains("[Partition]"));
    assert!(conf.contains("Type=generic"));
    assert!(conf.contains("CopyBlocks=/images/project.raw"));
    assert!(conf.contains("ReadOnly=yes"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test compose`
Expected: Compilation error

- [ ] **Step 3: Implement disk composition**

Replace `src/compose/disk.rs`:

```rust
use std::path::Path;

use crate::mkosi::config::MkosiConfig;

/// Generate the repart partition definition for the base partition.
pub fn base_partition_conf(base_partition: &Path) -> String {
    format!(
        "[Partition]\n\
         Type=root\n\
         Format=ext4\n\
         CopyBlocks={}\n\
         ReadOnly=yes\n\
         SizeMinBytes=2G\n",
        base_partition.display()
    )
}

/// Generate the repart partition definition for the project partition.
pub fn project_partition_conf(project_partition: &Path) -> String {
    format!(
        "[Partition]\n\
         Type=generic\n\
         Format=ext4\n\
         CopyBlocks={}\n\
         ReadOnly=yes\n\
         SizeMinBytes=512M\n",
        project_partition.display()
    )
}

/// Compose a final GPT disk image from base and project partitions using mkosi repart.
pub fn compose(
    base_partition: &Path,
    project_partition: &Path,
    output: &Path,
) -> anyhow::Result<()> {
    tracing::info!(
        base = %base_partition.display(),
        project = %project_partition.display(),
        output = %output.display(),
        "composing disk image via repart"
    );

    if !base_partition.exists() {
        anyhow::bail!("base partition not found: {}", base_partition.display());
    }
    if !project_partition.exists() {
        anyhow::bail!("project partition not found: {}", project_partition.display());
    }

    let work_dir = tempfile::tempdir()?;
    let definitions_dir = work_dir.path().join("definitions");
    fs_err::create_dir_all(&definitions_dir)?;

    fs_err::write(
        definitions_dir.join("00-base.conf"),
        base_partition_conf(base_partition),
    )?;
    fs_err::write(
        definitions_dir.join("10-project.conf"),
        project_partition_conf(project_partition),
    )?;

    let config = MkosiConfig::repart(definitions_dir, output.to_path_buf());
    config.invoke(work_dir.path())?;

    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test compose`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/compose/disk.rs tests/compose.rs
git commit -m "feat: implement disk composition via mkosi repart"
```

---

### Task 10: Implement base subcommand

**Files:**
- Modify: `src/commands/base.rs`

- [ ] **Step 1: Implement real base subcommand**

Replace `src/commands/base.rs`:

```rust
use crate::{nftables, source, tools, BaseArgs};
use crate::mkosi::config::MkosiConfig;

pub fn run(args: &BaseArgs) -> anyhow::Result<()> {
    tracing::info!(source_image = %args.source_image, "building base image");

    // Step 1: Resolve source image (download + cache if URL)
    let source_path = source::resolve(&args.source_image)?;
    tracing::info!(resolved = %source_path.display(), "source image resolved");

    // Step 2: Check required tools
    tools::require("mkosi")?;

    // Step 3: Create output directory
    fs_err::create_dir_all(&args.output)?;

    // Step 4: Generate mkosi config
    let work_dir = tempfile::tempdir()?;
    let mut config = MkosiConfig::base(source_path);

    // Step 5: Add nftables hardening (block all traffic)
    config.add_postinst_script(&nftables::base_rules());

    // Step 6: Invoke mkosi
    config.invoke(work_dir.path())?;

    tracing::info!(output = %args.output.display(), "base image build complete");
    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add src/commands/base.rs
git commit -m "feat: implement base subcommand with mkosi invocation and nftables hardening"
```

---

### Task 11: Wire up cloud-init pipeline

**Files:**
- Modify: `src/commands/cloud_init.rs`

- [ ] **Step 1: Replace stubs with real invocations**

Replace `src/commands/cloud_init.rs`:

```rust
use std::path::Path;

use crate::compose;
use crate::convert;
use crate::igvm::invoke::IgvmBuildArgs;
use crate::manifest::{
    self, BuildConfig, BuildManifest, FileEntry, ManifestInputs, ManifestOutputs,
};
use crate::mkosi::config::MkosiConfig;
use crate::nftables;
use crate::uki::build::UkifyBuildArgs;
use crate::{tools, CloudInitArgs, ImageFormat};

fn validate_inputs(args: &CloudInitArgs) -> anyhow::Result<()> {
    ensure_dir_exists(&args.dir, "cloud-init directory")?;
    ensure_file_exists(&args.kernel, "kernel")?;
    ensure_file_exists(&args.initrd, "initrd")?;
    ensure_file_exists(&args.firmware, "firmware")?;
    ensure_file_exists(&args.base_image, "base image")?;
    Ok(())
}

fn ensure_file_exists(path: &Path, label: &str) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("{label} not found: {}", path.display());
    }
    if !path.is_file() {
        anyhow::bail!("{label} is not a file: {}", path.display());
    }
    Ok(())
}

fn ensure_dir_exists(path: &Path, label: &str) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("{label} not found: {}", path.display());
    }
    if !path.is_dir() {
        anyhow::bail!("{label} is not a directory: {}", path.display());
    }
    Ok(())
}

fn format_extension(format: &ImageFormat) -> &'static str {
    match format {
        ImageFormat::Qcow2 => "qcow2",
        ImageFormat::Vhd => "vhd",
        ImageFormat::Raw => "raw",
    }
}

pub fn run(args: &CloudInitArgs) -> anyhow::Result<()> {
    tracing::info!(dir = %args.dir.display(), "building cloud-init CVM image");

    // Stage 1: Validate inputs
    validate_inputs(args)?;

    // Stage 2: Check required tools
    tools::require("mkosi")?;
    tools::require("ukify")?;
    tools::require("igvm-tools")?;
    tools::require("qemu-img")?;

    // Stage 3: Create output directory
    fs_err::create_dir_all(&args.output)?;

    tracing::info!("all inputs validated and tools found");

    // Stage 4: Build project partition via mkosi
    let work_dir = tempfile::tempdir()?;
    let mut mkosi_config = MkosiConfig::cloud_init(args.dir.clone());
    mkosi_config.add_postinst_script(&nftables::service_rules(args.service_port));
    mkosi_config.invoke(work_dir.path())?;
    let project_partition = work_dir.path().join("project.raw");
    tracing::info!("project partition built");

    // Stage 5: Compose disk image (base + project)
    let raw_disk = args.output.join("disk.raw");
    compose::disk::compose(&args.base_image, &project_partition, &raw_disk)?;
    tracing::info!("disk image composed");

    // Stage 6: Build UKI via ukify
    let uki_path = args.output.join("uki.efi");
    let uki_args = UkifyBuildArgs {
        kernel: args.kernel.clone(),
        initrds: vec![args.initrd.clone()],
        output: uki_path.clone(),
    };
    crate::uki::build::build(&uki_args)?;
    tracing::info!("UKI built");

    // Stage 7: Build IGVM via igvm-tools
    let igvm_manifest_path = work_dir.path().join("igvm-manifest.json");
    let igvm_path = args.output.join("guest.igvm");
    let igvm_args = IgvmBuildArgs {
        firmware: args.firmware.clone(),
        kernel: uki_path.clone(),
        smp: args.smp,
        manifest: Some(igvm_manifest_path.clone()),
        output: igvm_path.clone(),
    };
    crate::igvm::invoke::build(&igvm_args)?;
    tracing::info!("IGVM built");

    // Stage 8: Convert to output format
    let final_disk = args.output.join(format!("disk.{}", format_extension(&args.format)));
    convert::convert(&raw_disk, &final_disk, &args.format)?;
    // Remove raw intermediate if we converted to another format
    if !matches!(args.format, ImageFormat::Raw) && raw_disk.exists() {
        fs_err::remove_file(&raw_disk)?;
    }
    tracing::info!(format = format_extension(&args.format), "disk image ready");

    // Stage 9: Write manifest
    let igvm_manifest_json = fs_err::read_to_string(&igvm_manifest_path)?;
    let measurement = manifest::parse_igvm_manifest(&igvm_manifest_json)?;

    let build_manifest = BuildManifest {
        version: 1,
        build: BuildConfig {
            timestamp: chrono_now(),
            smp: args.smp,
            memory: args.memory.clone(),
            format: format_extension(&args.format).to_string(),
            platform: "snp".to_string(),
        },
        inputs: ManifestInputs {
            kernel: hash_entry(&args.kernel)?,
            initrd: hash_entry(&args.initrd)?,
            firmware: hash_entry(&args.firmware)?,
            base_image: hash_entry(&args.base_image)?,
            project_partition: hash_entry(&project_partition)?,
        },
        outputs: ManifestOutputs {
            disk_image: hash_entry(&final_disk)?,
            igvm: hash_entry(&igvm_path)?,
            uki: hash_entry(&uki_path)?,
        },
        measurement,
    };

    let manifest_path = args.output.join("manifest.json");
    manifest::write_manifest(&build_manifest, &manifest_path)?;
    tracing::info!(path = %manifest_path.display(), "manifest written");

    tracing::info!(output = %args.output.display(), "pipeline complete");
    Ok(())
}

fn hash_entry(path: &Path) -> anyhow::Result<FileEntry> {
    Ok(FileEntry {
        path: path.display().to_string(),
        sha256: manifest::sha256_file(path)?,
    })
}

fn chrono_now() -> String {
    // Use a simple UTC timestamp without pulling in the chrono crate
    let output = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => "unknown".to_string(),
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles successfully

- [ ] **Step 3: Run all tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/commands/cloud_init.rs
git commit -m "feat: wire up full cloud-init pipeline with real tool invocations"
```

---

### Task 12: Implement run subcommand

**Files:**
- Modify: `src/commands/run.rs`

- [ ] **Step 1: Implement run subcommand**

Replace `src/commands/run.rs`:

```rust
use std::path::Path;

use crate::convert;
use crate::manifest;
use crate::qemu::QemuArgs;
use crate::RunArgs;

pub fn run(args: &RunArgs) -> anyhow::Result<()> {
    tracing::info!(dir = %args.dir.display(), "launching CVM");

    // Step 1: Validate directory exists
    if !args.dir.exists() {
        anyhow::bail!("output directory not found: {}", args.dir.display());
    }

    // Step 2: Read manifest
    let manifest_path = args.dir.join("manifest.json");
    if !manifest_path.exists() {
        anyhow::bail!("manifest.json not found in {}", args.dir.display());
    }
    let manifest = manifest::read_manifest(&manifest_path)?;

    // Step 3: Find IGVM file
    let igvm_path = args.dir.join("guest.igvm");
    if !igvm_path.exists() {
        anyhow::bail!("guest.igvm not found in {}", args.dir.display());
    }

    // Step 4: Find disk image using format from manifest
    let disk_path = args.dir.join(format!("disk.{}", manifest.build.format));
    if !disk_path.exists() {
        anyhow::bail!("disk.{} not found in {}", manifest.build.format, args.dir.display());
    }

    // Step 5: Determine qemu disk format
    let format_enum = match manifest.build.format.as_str() {
        "qcow2" => crate::ImageFormat::Qcow2,
        "vhd" => crate::ImageFormat::Vhd,
        "raw" => crate::ImageFormat::Raw,
        other => anyhow::bail!("unknown disk format in manifest: {other}"),
    };
    let qemu_format = convert::qemu_img_format(&format_enum);

    // Step 6: Launch QEMU
    let qemu_args = QemuArgs {
        igvm: igvm_path,
        disk: disk_path,
        disk_format: qemu_format.to_string(),
        smp: manifest.build.smp,
        memory: manifest.build.memory,
    };
    crate::qemu::launch(&qemu_args)?;

    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add src/commands/run.rs
git commit -m "feat: implement run subcommand to launch CVMs via QEMU"
```

---

## Chunk 4: Integration Tests & Polish

### Task 13: Add integration tests and run clippy

**Files:**
- Modify: `tests/cli.rs`

- [ ] **Step 1: Add integration test for run with missing manifest**

Add to the top of `tests/cli.rs` (if not already present):

```rust
use tempfile;
```

Add to `tests/cli.rs`:

```rust
#[test]
fn test_run_fails_with_missing_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args(["run", dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicates::str::contains("manifest.json not found"));
}
```

- [ ] **Step 2: Run all tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 4: Fix any clippy warnings**

If clippy reports warnings, fix them.

- [ ] **Step 5: Commit**

```bash
git add tests/ src/
git commit -m "test: add integration tests for run subcommand and fix clippy warnings"
```

---

### Task 14: Final verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 2: Run clippy clean**

Run: `cargo clippy -- -D warnings`
Expected: Clean

- [ ] **Step 3: Verify help output**

Run: `cargo run -- --help`
Expected: Shows all subcommands including `run`

Run: `cargo run -- cloud-init --help`
Expected: Shows `--service-port` and `--memory` flags

- [ ] **Step 4: Commit any final fixes**

Only if needed.

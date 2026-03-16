# steep container Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `steep container` to build confidential VM images from OCI container references, with podman bake-in and systemd quadlet startup.

**Architecture:** Extract shared pipeline stages 5-9 from cloud_init.rs into a reusable pipeline module. Add container-specific helpers (podman pull/save, quadlet generation). The container command builds its project partition via mkosi with baked-in OCI image and quadlet, then calls the shared pipeline.

**Tech Stack:** Rust, clap (derive), tracing, serde, sha2, fs_err, thiserror, tempfile. External tools: mkosi, podman, systemd-ukify, igvm-tools, qemu-img.

**Spec:** `docs/superpowers/specs/2026-03-16-steep-container.md`

---

## File Map

| File | Responsibility |
|------|---------------|
| `src/pipeline.rs` | Shared pipeline stages 5-9: compose disk, build UKI, build IGVM, convert format, write manifest |
| `src/container.rs` | Container helpers: podman pull/save, quadlet generation, postinst script generation |
| `src/lib.rs` | Add `service_port` + `memory` to `ContainerArgs`; add `pub mod pipeline;` + `pub mod container;` |
| `src/mkosi/config.rs` | Add `Container` variant, `container()` constructor, `extra_files` field, `add_extra_file()`, `write_extra_files()` |
| `src/commands/cloud_init.rs` | Replace inline stages 5-9 with `pipeline::run()` call |
| `src/commands/container.rs` | Replace stub with full orchestration |

---

## Chunk 1: CLI Updates & Pipeline Extraction

### Task 1: Add service_port and memory to ContainerArgs

**Files:**
- Modify: `src/lib.rs:88-120`
- Modify: `tests/cli.rs`

- [ ] **Step 1: Write tests for new container CLI args**

Add to `tests/cli.rs`:

```rust
#[test]
fn test_container_requires_service_port() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args([
        "container", "ghcr.io/org/app:latest",
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
fn test_container_accepts_service_port_and_memory() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args([
        "container", "ghcr.io/org/app:latest",
        "--kernel", "/tmp/k",
        "--initrd", "/tmp/i",
        "--firmware", "/tmp/f",
        "--base-image", "/tmp/b",
        "--service-port", "8080",
        "--memory", "4G",
        "-o", "/tmp/o",
    ])
    .assert()
    .failure(); // Fails on validation, not parsing
}

#[test]
fn test_container_memory_default() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args([
        "container", "ghcr.io/org/app:latest",
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test cli test_container_requires_service_port test_container_accepts_service_port_and_memory test_container_memory_default`
Expected: Compilation error — `--service-port` not recognized by container subcommand

- [ ] **Step 3: Add service_port and memory to ContainerArgs**

In `src/lib.rs`, add two fields to the `ContainerArgs` struct, after `base_image` and before `smp`:

```rust
    /// Single TCP port to allow through firewall
    #[arg(long)]
    pub service_port: u16,

    /// RAM for VM (QEMU-style suffix, e.g. "2G")
    #[arg(long, default_value = "2G")]
    pub memory: String,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test cli`
Expected: All CLI tests pass

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs tests/cli.rs
git commit -m "feat: add --service-port and --memory flags to container subcommand"
```

---

### Task 2: Extract shared pipeline module

**Files:**
- Create: `src/pipeline.rs`
- Modify: `src/lib.rs`
- Modify: `src/commands/cloud_init.rs`

- [ ] **Step 1: Create pipeline module with PipelineArgs struct and run function**

Create `src/pipeline.rs`:

```rust
use std::path::Path;

use crate::compose;
use crate::convert;
use crate::igvm::invoke::IgvmBuildArgs;
use crate::manifest::{
    self, BuildConfig, BuildManifest, FileEntry, ManifestInputs, ManifestOutputs,
};
use crate::uki::build::UkifyBuildArgs;
use crate::ImageFormat;

pub struct PipelineArgs {
    pub project_partition: std::path::PathBuf,
    pub kernel: std::path::PathBuf,
    pub initrd: std::path::PathBuf,
    pub firmware: std::path::PathBuf,
    pub base_image: std::path::PathBuf,
    pub memory: String,
    pub smp: u32,
    pub format: ImageFormat,
    pub output: std::path::PathBuf,
}

pub fn format_extension(format: &ImageFormat) -> &'static str {
    match format {
        ImageFormat::Qcow2 => "qcow2",
        ImageFormat::Vhd => "vhd",
        ImageFormat::Raw => "raw",
    }
}

pub fn run(args: &PipelineArgs) -> anyhow::Result<()> {
    // Stage 5: Compose disk image (base + project)
    let raw_disk = args.output.join("disk.raw");
    compose::disk::compose(&args.base_image, &args.project_partition, &raw_disk)?;
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
    let igvm_work_dir = tempfile::tempdir()?;
    let igvm_manifest_path = igvm_work_dir.path().join("igvm-manifest.json");
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
            project_partition: hash_entry(&args.project_partition)?,
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
    let output = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => "unknown".to_string(),
    }
}
```

- [ ] **Step 2: Add pipeline module declaration to lib.rs**

In `src/lib.rs`, add after the `pub mod nftables;` line:

```rust
pub mod pipeline;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check`
Expected: Compiles cleanly

- [ ] **Step 4: Refactor cloud_init.rs to use pipeline::run()**

Replace the contents of `src/commands/cloud_init.rs` with:

```rust
use std::path::Path;

use crate::mkosi::config::MkosiConfig;
use crate::nftables;
use crate::pipeline::{self, PipelineArgs};
use crate::{tools, CloudInitArgs};

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

    // Stages 5-9: Shared pipeline
    pipeline::run(&PipelineArgs {
        project_partition,
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

- [ ] **Step 5: Run all tests to verify refactor is behavior-preserving**

Run: `cargo test`
Expected: All existing tests pass — no behavior change

- [ ] **Step 6: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 7: Commit**

```bash
git add src/pipeline.rs src/lib.rs src/commands/cloud_init.rs
git commit -m "refactor: extract shared pipeline stages 5-9 into pipeline module"
```

---

## Chunk 2: MkosiConfig Extensions

### Task 3: Add Container variant and constructor to MkosiConfig

**Files:**
- Modify: `src/mkosi/config.rs`
- Modify: `tests/mkosi_config.rs`

- [ ] **Step 1: Write tests for container config**

Add to `tests/mkosi_config.rs`:

```rust
#[test]
fn test_container_config_profile() {
    let config = MkosiConfig::container();
    assert_eq!(config.profile, MkosiProfile::Container);
}

#[test]
fn test_container_config_ini() {
    let config = MkosiConfig::container();
    let ini = config.to_ini();
    assert!(ini.contains("[Distribution]"));
    assert!(ini.contains("Distribution=ubuntu"));
    assert!(ini.contains("[Content]"));
    assert!(ini.contains("Packages=podman"));
    assert!(ini.contains("[Output]"));
    assert!(ini.contains("Format=disk"));
}

#[test]
fn test_container_config_has_no_cloud_init_dir() {
    let config = MkosiConfig::container();
    assert!(config.cloud_init_dir.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test mkosi_config test_container`
Expected: Compilation error — `MkosiConfig::container()` and `MkosiProfile::Container` not defined

- [ ] **Step 3: Add Container variant to MkosiProfile**

In `src/mkosi/config.rs`, update the `MkosiProfile` enum:

```rust
#[derive(Debug, PartialEq)]
pub enum MkosiProfile {
    Base,
    CloudInit,
    Container,
    Repart,
}
```

- [ ] **Step 4: Add container() constructor to MkosiConfig**

In `src/mkosi/config.rs`, add after the `cloud_init()` method. Use the existing struct shape (without `extra_files` — that field is added in Task 4):

```rust
    /// Create a mkosi config for building a container project partition.
    pub fn container() -> Self {
        let mut config = Self {
            profile: MkosiProfile::Container,
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
            vec![("Packages".to_string(), "podman".to_string())],
        ));
        config.sections.push((
            "Output".to_string(),
            vec![("Format".to_string(), "disk".to_string())],
        ));
        config
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --test mkosi_config`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/mkosi/config.rs tests/mkosi_config.rs
git commit -m "feat: add Container variant and constructor to MkosiConfig"
```

---

### Task 4: Add extra_files support to MkosiConfig

**Files:**
- Modify: `src/mkosi/config.rs`
- Modify: `tests/mkosi_config.rs`

- [ ] **Step 1: Write tests for extra_files**

Add to `tests/mkosi_config.rs`:

```rust
#[test]
fn test_add_extra_file() {
    let mut config = MkosiConfig::container();
    config.add_extra_file(
        std::path::PathBuf::from("etc/containers/systemd/app.container"),
        b"[Container]\nImage=test\n".to_vec(),
    );
    assert_eq!(config.extra_files.len(), 1);
    assert_eq!(config.extra_files[0].0, std::path::PathBuf::from("etc/containers/systemd/app.container"));
}

#[test]
fn test_write_extra_files() {
    let mut config = MkosiConfig::container();
    config.add_extra_file(
        std::path::PathBuf::from("etc/myfile.conf"),
        b"content".to_vec(),
    );
    let dir = tempfile::tempdir().unwrap();
    config.write_extra_files(dir.path()).unwrap();
    let written = std::fs::read_to_string(dir.path().join("mkosi.extra/etc/myfile.conf")).unwrap();
    assert_eq!(written, "content");
}
```

Add `use tempfile;` at the top of the test file if not already present.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test mkosi_config test_add_extra_file test_write_extra_files`
Expected: Compilation error — `extra_files` field and `add_extra_file`/`write_extra_files` methods not defined

- [ ] **Step 3: Add extra_files field to MkosiConfig**

In `src/mkosi/config.rs`, update the `MkosiConfig` struct:

```rust
pub struct MkosiConfig {
    pub profile: MkosiProfile,
    pub source_image: Option<PathBuf>,
    pub cloud_init_dir: Option<PathBuf>,
    pub postinst_scripts: Vec<String>,
    pub extra_files: Vec<(PathBuf, Vec<u8>)>,
    sections: Vec<(String, Vec<(String, String)>)>,
}
```

Update all existing constructors (`base()`, `cloud_init()`, `repart()`, `container()`) to initialize `extra_files: Vec::new()`.

- [ ] **Step 4: Add add_extra_file and write_extra_files methods**

In `src/mkosi/config.rs`, add to the `impl MkosiConfig` block:

```rust
    /// Add a file to be written into the mkosi.extra/ tree.
    /// The path is relative to the image root (e.g., "etc/containers/systemd/app.container").
    pub fn add_extra_file(&mut self, relative_path: PathBuf, content: Vec<u8>) {
        self.extra_files.push((relative_path, content));
    }

    /// Write extra files to the mkosi.extra/ directory in the build tree.
    pub fn write_extra_files(&self, build_dir: &std::path::Path) -> anyhow::Result<()> {
        if self.extra_files.is_empty() {
            return Ok(());
        }
        let extra_dir = build_dir.join("mkosi.extra");
        for (relative_path, content) in &self.extra_files {
            let dest = extra_dir.join(relative_path);
            if let Some(parent) = dest.parent() {
                fs_err::create_dir_all(parent)?;
            }
            fs_err::write(&dest, content)?;
        }
        Ok(())
    }
```

- [ ] **Step 5: Update invoke() to call write_extra_files()**

In `src/mkosi/config.rs`, update the `invoke()` method to call `write_extra_files` after `write_postinst_scripts`:

```rust
    pub fn invoke(&self, work_dir: &std::path::Path) -> anyhow::Result<()> {
        let config_path = work_dir.join("mkosi.conf");
        self.write_to(&config_path)?;
        self.write_postinst_scripts(work_dir)?;
        self.write_extra_files(work_dir)?;
        crate::tools::require("mkosi")?;
        let args = self.to_mkosi_args(work_dir);
        tracing::info!(config = %config_path.display(), "invoking mkosi");
        crate::tools::run_command_streaming("mkosi", &args)?;
        Ok(())
    }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --test mkosi_config`
Expected: All tests pass

- [ ] **Step 7: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 8: Commit**

```bash
git add src/mkosi/config.rs tests/mkosi_config.rs
git commit -m "feat: add extra_files support to MkosiConfig for mkosi.extra/ tree"
```

---

## Chunk 3: Container Helpers

### Task 5: Implement container helpers module

**Files:**
- Create: `src/container.rs`
- Create: `tests/container.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write tests for container helpers**

Create `tests/container.rs`:

```rust
use steep::container;

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

#[test]
fn test_podman_postinst_installs_podman() {
    let script = container::podman_postinst();
    assert!(script.contains("apt-get install -y podman"));
}

#[test]
fn test_podman_postinst_loads_image() {
    let script = container::podman_postinst();
    assert!(script.contains("podman load -i /opt/steep/container.oci"));
}

#[test]
fn test_podman_postinst_removes_archive() {
    let script = container::podman_postinst();
    assert!(script.contains("rm /opt/steep/container.oci"));
}

#[test]
fn test_podman_postinst_starts_with_shebang() {
    let script = container::podman_postinst();
    assert!(script.starts_with("#!/bin/bash\n"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test container`
Expected: Compilation error — `container` module not defined

- [ ] **Step 3: Create container module**

Create `src/container.rs`:

```rust
use std::path::Path;

use crate::tools;

/// Pull an OCI container image using podman.
pub fn pull(url: &str) -> anyhow::Result<()> {
    tracing::info!(url = url, "pulling container image");
    tools::run_command_streaming("podman", &["pull", url])?;
    Ok(())
}

/// Save a container image to an OCI archive.
pub fn save(url: &str, dest: &Path) -> anyhow::Result<()> {
    let dest_str = dest.display().to_string();
    tracing::info!(url = url, dest = %dest_str, "saving container image to archive");
    tools::run_command_streaming("podman", &["save", "-o", &dest_str, url])?;
    Ok(())
}

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

/// Generate the postinst script that installs podman and loads the baked OCI image.
/// The OCI archive is at the fixed path /opt/steep/container.oci inside the chroot.
pub fn podman_postinst() -> String {
    "#!/bin/bash\n\
     set -euo pipefail\n\
     apt-get install -y podman\n\
     podman load -i /opt/steep/container.oci\n\
     rm /opt/steep/container.oci\n"
        .to_string()
}
```

- [ ] **Step 4: Add module declaration to lib.rs**

In `src/lib.rs`, add after `pub mod compose;`:

```rust
pub mod container;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --test container`
Expected: All 8 tests pass

- [ ] **Step 6: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 7: Commit**

```bash
git add src/container.rs src/lib.rs tests/container.rs
git commit -m "feat: implement container helpers for podman pull/save, quadlet, and postinst"
```

---

## Chunk 4: Container Command Implementation

### Task 6: Implement steep container command

**Files:**
- Modify: `src/commands/container.rs`
- Modify: `tests/cli.rs`

- [ ] **Step 1: Write integration test for container validation**

Add to `tests/cli.rs`:

```rust
#[test]
fn test_container_fails_with_missing_kernel() {
    let mut cmd = Command::cargo_bin("steep").unwrap();
    cmd.args([
        "container", "ghcr.io/org/app:latest",
        "--kernel", "/nonexistent/kernel",
        "--initrd", "/tmp/i",
        "--firmware", "/tmp/f",
        "--base-image", "/tmp/b",
        "--service-port", "8080",
        "-o", "/tmp/o",
    ])
    .assert()
    .failure()
    .stderr(predicates::str::contains("not found"));
}
```

- [ ] **Step 2: Replace container.rs stub with full implementation**

Replace the contents of `src/commands/container.rs` with:

```rust
use std::path::{Path, PathBuf};

use crate::container as container_helpers;
use crate::mkosi::config::MkosiConfig;
use crate::nftables;
use crate::pipeline::{self, PipelineArgs};
use crate::{tools, ContainerArgs};

fn validate_inputs(args: &ContainerArgs) -> anyhow::Result<()> {
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

pub fn run(args: &ContainerArgs) -> anyhow::Result<()> {
    tracing::info!(url = %args.url, "building container CVM image");

    // Stage 1: Validate inputs
    validate_inputs(args)?;

    // Stage 2: Check required tools
    tools::require("mkosi")?;
    tools::require("ukify")?;
    tools::require("igvm-tools")?;
    tools::require("qemu-img")?;
    tools::require("podman")?;

    // Stage 3: Create output directory
    fs_err::create_dir_all(&args.output)?;

    tracing::info!("all inputs validated and tools found");

    // Stage 4: Build project partition
    let work_dir = tempfile::tempdir()?;

    // 4a: Pull and export OCI image
    container_helpers::pull(&args.url)?;
    let oci_archive = work_dir.path().join("container.oci");
    container_helpers::save(&args.url, &oci_archive)?;
    tracing::info!("container image exported");

    // 4b: Generate mkosi build tree
    let mut mkosi_config = MkosiConfig::container();

    // Postinst scripts: nftables first (index 0), podman second (index 1)
    mkosi_config.add_postinst_script(&nftables::service_rules(args.service_port));
    mkosi_config.add_postinst_script(&container_helpers::podman_postinst());

    // Small extra file: quadlet unit
    mkosi_config.add_extra_file(
        PathBuf::from("etc/containers/systemd/app.container"),
        container_helpers::quadlet(&args.url, args.service_port).into_bytes(),
    );

    // Large extra file: OCI archive — copy directly to avoid loading into memory
    let extra_oci_dir = work_dir.path().join("mkosi.extra/opt/steep");
    fs_err::create_dir_all(&extra_oci_dir)?;
    fs_err::copy(&oci_archive, extra_oci_dir.join("container.oci"))?;

    mkosi_config.invoke(work_dir.path())?;
    let project_partition = work_dir.path().join("project.raw");
    tracing::info!("project partition built");

    // Stages 5-9: Shared pipeline
    pipeline::run(&PipelineArgs {
        project_partition,
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

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 4: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 5: Commit**

```bash
git add src/commands/container.rs tests/cli.rs
git commit -m "feat: implement steep container command with podman bake-in and quadlet"
```

---

### Task 7: Final verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Verify help output shows all flags**

Run: `cargo run -- container --help`
Expected: Shows `--service-port`, `--memory`, `--kernel`, `--initrd`, `--firmware`, `--base-image`, `--smp`, `--format`, `-o`

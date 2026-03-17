# Static mkosi Base Config Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Rust-generated mkosi config for `steep base` with a static folder of committed config files, invoked directly by mkosi.

**Architecture:** Create `mkosi/base/` with `mkosi.conf` and a nftables postinst script. Update `src/commands/base.rs` to invoke mkosi with `--directory` pointing at this folder and `--output-dir` pointing at a temp dir. Remove `MkosiConfig::base()`, `MkosiProfile::Base`, and `nftables::base_rules()` along with their tests.

**Tech Stack:** Rust (cargo), mkosi (INI config), nftables

**Spec:** `docs/superpowers/specs/2026-03-17-mkosi-static-base-config-design.md`

---

## File Map

| Action | Path | Change |
|--------|------|--------|
| Create | `mkosi/base/mkosi.conf` | Static mkosi configuration |
| Create | `mkosi/base/mkosi.postinst.d/00-nftables.sh` | nftables base rules (executable) |
| Modify | `.gitignore` | Add mkosi cache dir |
| Modify | `src/commands/base.rs` | Replace MkosiConfig usage with direct mkosi invocation |
| Modify | `src/mkosi/config.rs` | Remove `MkosiProfile::Base` and `MkosiConfig::base()` |
| Modify | `src/nftables.rs` | Remove `base_rules()` |
| Modify | `tests/mkosi_config.rs` | Remove 4 tests referencing base config |
| Modify | `tests/nftables.rs` | Remove 5 tests referencing `base_rules()` |

---

### Task 1: Create static mkosi config folder

**Files:**
- Create: `mkosi/base/mkosi.conf`
- Create: `mkosi/base/mkosi.postinst.d/00-nftables.sh`
- Modify: `.gitignore`

- [ ] **Step 1: Create mkosi/base/mkosi.conf**

  Create the file with this exact content:

  ```ini
  [Distribution]
  Distribution=ubuntu

  [Output]
  Format=disk
  Output=image.raw
  ```

- [ ] **Step 2: Create mkosi/base/mkosi.postinst.d/00-nftables.sh**

  Create the file with this exact content:

  ```
  #!/usr/sbin/nft -f
  flush ruleset
  table inet filter {
      chain input {
          type filter hook input priority 0; policy drop;
          iif "lo" accept
          ct state established,related accept
      }
      chain forward {
          type filter hook forward priority 0; policy drop;
      }
      chain output {
          type filter hook output priority 0; policy drop;
          oif "lo" accept
          ct state established,related accept
      }
  }
  ```

  Then mark it executable in both the filesystem and git:

  ```bash
  chmod +x mkosi/base/mkosi.postinst.d/00-nftables.sh
  git update-index --chmod=+x mkosi/base/mkosi.postinst.d/00-nftables.sh
  ```

- [ ] **Step 3: Add mkosi cache to .gitignore**

  Append to `.gitignore`:

  ```
  mkosi.cache/
  ```

- [ ] **Step 4: Verify build is still green**

  ```bash
  cargo build
  ```

  Expected: compiles with no errors.

- [ ] **Step 5: Commit**

  ```bash
  git add mkosi/base/ .gitignore
  git commit -m "feat: add static mkosi config folder for base image"
  ```

---

### Task 2: Update base.rs to use the static folder

**Files:**
- Modify: `src/commands/base.rs`

- [ ] **Step 1: Replace the entire file content**

  Replace `src/commands/base.rs` with:

  ```rust
  use std::path::PathBuf;

  use crate::{tools, BaseArgs};

  pub fn run(args: &BaseArgs) -> anyhow::Result<()> {
      tracing::info!("building base image");

      // Step 1: Check required tools
      tools::require("mkosi")?;

      // Step 2: Create output directory
      fs_err::create_dir_all(&args.output)?;

      // Step 3: Invoke mkosi against static config folder
      let mkosi_dir = PathBuf::from("mkosi/base");
      if !mkosi_dir.exists() {
          anyhow::bail!("mkosi config dir not found: {}", mkosi_dir.display());
      }

      let output_dir = tempfile::tempdir()?;
      tracing::info!(config = %mkosi_dir.display(), "invoking mkosi");
      tools::run_command_streaming("mkosi", &[
          "--directory",
          mkosi_dir.to_str().unwrap(),
          "--output-dir",
          output_dir.path().to_str().unwrap(),
          "build",
      ])?;

      // Step 4: Copy mkosi output to args.output/base.raw
      let mkosi_output = output_dir.path().join("image.raw");
      let dest = args.output.join("base.raw");
      fs_err::copy(&mkosi_output, &dest)?;
      tracing::info!(dest = %dest.display(), "base image written");

      tracing::info!(output = %args.output.display(), "base image build complete");
      Ok(())
  }
  ```

- [ ] **Step 2: Verify build is still green**

  ```bash
  cargo build
  ```

  Expected: compiles with no errors. `MkosiConfig::base()` and `nftables::base_rules()` are now dead code but still compile — no errors yet.

- [ ] **Step 3: Commit**

  ```bash
  git add src/commands/base.rs
  git commit -m "feat: invoke mkosi directly against static config folder in base command"
  ```

---

### Task 3: Remove Rust base config code

**Files:**
- Modify: `src/mkosi/config.rs`
- Modify: `src/nftables.rs`

- [ ] **Step 1: Remove MkosiProfile::Base from src/mkosi/config.rs**

  In `src/mkosi/config.rs`, change the `MkosiProfile` enum (lines 5–10) from:

  ```rust
  pub enum MkosiProfile {
      Base,
      CloudInit,
      Container,
      Repart,
  }
  ```

  to:

  ```rust
  pub enum MkosiProfile {
      CloudInit,
      Container,
      Repart,
  }
  ```

- [ ] **Step 2: Remove MkosiConfig::base() from src/mkosi/config.rs**

  Remove the entire `base()` method and its doc comment (lines 22–43):

  ```rust
      /// Create a mkosi config for building the base partition.
      pub fn base() -> Self {
          let mut config = Self {
              profile: MkosiProfile::Base,
              cloud_init_dir: None,
              postinst_scripts: Vec::new(),
              extra_files: Vec::new(),
              sections: Vec::new(),
          };
          config.sections.push((
              "Distribution".to_string(),
              vec![("Distribution".to_string(), "ubuntu".to_string())],
          ));
          config.sections.push((
              "Output".to_string(),
              vec![
                  ("Format".to_string(), "disk".to_string()),
                  ("Output".to_string(), "image.raw".to_string()),
              ],
          ));
          config
      }
  ```

- [ ] **Step 3: Remove nftables::base_rules() from src/nftables.rs**

  Remove the entire `base_rules` function and its doc comment (lines 1–6 of `src/nftables.rs`):

  ```rust
  /// Generate nftables rules for the base image.
  /// Blocks all new incoming and outgoing connections.
  /// Only loopback and already-established connections are permitted.
  pub fn base_rules() -> String {
      "#!/usr/sbin/nft -f\nflush ruleset\n...".to_string()
  }
  ```

  Also remove the blank line that followed it so `service_rules` begins at line 1.

- [ ] **Step 4: Verify build**

  ```bash
  cargo build
  ```

  Expected: compiles with no errors.

- [ ] **Step 5: Commit**

  ```bash
  git add src/mkosi/config.rs src/nftables.rs
  git commit -m "remove: MkosiConfig::base(), MkosiProfile::Base, nftables::base_rules()"
  ```

---

### Task 4: Remove stale tests and verify

**Files:**
- Modify: `tests/mkosi_config.rs`
- Modify: `tests/nftables.rs`

- [ ] **Step 1: Remove 4 tests from tests/mkosi_config.rs**

  Remove these four functions entirely (the `use` imports at the top are still needed by remaining tests):

  - `test_base_config_generates_valid_ini` (lines 4–10)
  - `test_config_profile` (lines 19–23)
  - `test_add_postinst_script` (lines 25–31)
  - `test_invoke_args_base` (lines 44–50)

  Leave untouched: `test_cloud_init_config_includes_cloud_init_dir`, `test_repart_config`, `test_container_config_profile`, `test_container_config_ini`, `test_container_config_has_no_cloud_init_dir`, `test_add_extra_file`, `test_write_extra_files`.

  Note: `use std::path::PathBuf` and `use steep::mkosi::config::{MkosiConfig, MkosiProfile}` are still needed — remaining tests use `PathBuf`, `MkosiConfig::cloud_init/repart/container`, and `MkosiProfile::Container`.

- [ ] **Step 2: Remove 5 tests from tests/nftables.rs**

  Remove these five functions entirely:

  - `test_base_rules_drops_all_input` (lines 3–10)
  - `test_base_rules_allows_loopback` (lines 12–17)
  - `test_base_rules_allows_established` (lines 19–23)
  - `test_base_rules_output_policy_is_drop` (lines 25–29)
  - `test_base_rules_starts_with_shebang` (lines 49–53)

  Leave untouched: `test_service_rules_opens_port`, `test_service_rules_output_policy_is_accept`, `test_service_rules_starts_with_shebang`.

- [ ] **Step 3: Run all tests**

  ```bash
  cargo test
  ```

  Expected: all tests pass, no compilation errors, no test failures.

- [ ] **Step 4: Commit**

  ```bash
  git add tests/mkosi_config.rs tests/nftables.rs
  git commit -m "remove: stale tests for base config Rust code"
  ```

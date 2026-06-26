# Chart Module Run Subcommand Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `chartwright run <library-path>` so generated chart dynamic libraries can be loaded and rendered from the CLI for testing.

**Architecture:** The CLI crate will depend on `chartwright-abi` and reuse `LoadedChartModule` for safe dynamic loading. Testable behavior will live in `chartwright_cli::run_chart_module`; the binary will parse flags, build a `RenderRequest`, call the helper, and print the rendered manifest.

**Tech Stack:** Rust 2021, Clap, `chartwright-abi`, `serde_yaml`, `serde_json`, Cargo integration tests.

---

## File Structure

- Modify `crates/chartwright-cli/Cargo.toml` to add the `chartwright-abi` dependency.
- Modify `crates/chartwright-cli/src/lib.rs` to add `RunError`, `run_chart_module`, `values_from_file`, and a small path-aware read helper for values files.
- Modify `crates/chartwright-cli/src/main.rs` to add the `run` subcommand and stdout rendering.
- Modify `crates/chartwright-cli/tests/import.rs` to add tests for the helper and values-file parsing.

### Task 1: Library Helper

**Files:**
- Modify: `crates/chartwright-cli/Cargo.toml`
- Modify: `crates/chartwright-cli/src/lib.rs`
- Test: `crates/chartwright-cli/tests/import.rs`

- [ ] **Step 1: Write the failing helper test**

Add these imports and helper functions to `crates/chartwright-cli/tests/import.rs`:

```rust
use std::process::Command;

use chartwright_abi::RenderRequest;
```

```rust
fn dynamic_library_path(crate_dir: &std::path::Path, crate_name: &str) -> std::path::PathBuf {
    let file_name = if cfg!(target_os = "macos") {
        format!("lib{crate_name}.dylib")
    } else if cfg!(target_os = "windows") {
        format!("{crate_name}.dll")
    } else {
        format!("lib{crate_name}.so")
    };
    crate_dir.join("target/release").join(file_name)
}

fn build_generated_crate(crate_dir: &std::path::Path) {
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(crate_dir)
        .status()
        .unwrap();
    assert!(status.success());
}
```

Add this test:

```rust
#[test]
fn run_chart_module_renders_generated_library() {
    let temp = tempfile::tempdir().unwrap();
    let out_dir = temp.path().join("generated-basic-chart");
    import_chart("../../fixtures/basic-chart", &out_dir).unwrap();
    build_generated_crate(&out_dir);

    let rendered = chartwright_cli::run_chart_module(
        dynamic_library_path(&out_dir, "basic_chart"),
        RenderRequest {
            release_name: "demo".to_owned(),
            namespace: "testing".to_owned(),
            values: serde_json::json!({}),
            kube_version: "1.30.0".to_owned(),
            api_versions: vec!["v1".to_owned()],
        },
    )
    .unwrap();

    assert_eq!(
        rendered,
        std::fs::read_to_string("../../fixtures/basic-chart/golden.yaml").unwrap()
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p chartwright-cli run_chart_module_renders_generated_library
```

Expected: FAIL because `chartwright_abi` is not available to the CLI test target or `run_chart_module` is not defined.

- [ ] **Step 3: Implement the helper**

Add to `crates/chartwright-cli/Cargo.toml` dependencies:

```toml
chartwright-abi = { path = "../chartwright-abi" }
```

Add to `crates/chartwright-cli/src/lib.rs`:

```rust
use chartwright_abi::{LoadedChartModule, RenderRequest};
```

Extend `ImportError` area with:

```rust
#[derive(Debug, Error)]
pub enum RunError {
    #[error("module load or render failed: {0}")]
    Module(#[from] chartwright_abi::LoadError),
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid values yaml at {path}: {source}")]
    InvalidValuesYaml {
        path: String,
        source: serde_yaml::Error,
    },
    #[error("failed to convert values from yaml to json at {path}: {source}")]
    InvalidValuesJson {
        path: String,
        source: serde_json::Error,
    },
}

pub fn run_chart_module(
    library_path: impl AsRef<Path>,
    request: RenderRequest,
) -> Result<String, RunError> {
    let module = LoadedChartModule::load(library_path)?;
    module.render(request).map_err(RunError::from)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p chartwright-cli run_chart_module_renders_generated_library
```

Expected: PASS.

### Task 2: Values Parsing and CLI Wiring

**Files:**
- Modify: `crates/chartwright-cli/src/lib.rs`
- Modify: `crates/chartwright-cli/src/main.rs`
- Test: `crates/chartwright-cli/tests/import.rs`

- [ ] **Step 1: Write the failing values-file test**

Add to `crates/chartwright-cli/tests/import.rs`:

```rust
#[test]
fn values_from_file_parses_yaml_as_json_values() {
    let temp = tempfile::tempdir().unwrap();
    let values_path = temp.path().join("values.yaml");
    std::fs::write(
        &values_path,
        "replicaCount: 3\nimage:\n  repository: example/app\n",
    )
    .unwrap();

    let values = chartwright_cli::values_from_file(&values_path).unwrap();

    assert_eq!(values["replicaCount"], serde_json::json!(3));
    assert_eq!(values["image"]["repository"], serde_json::json!("example/app"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p chartwright-cli values_from_file_parses_yaml_as_json_values
```

Expected: FAIL because `values_from_file` is not defined.

- [ ] **Step 3: Implement values parsing**

Add to `crates/chartwright-cli/src/lib.rs`:

```rust
pub fn values_from_file(path: impl AsRef<Path>) -> Result<serde_json::Value, RunError> {
    let path = path.as_ref();
    let content = fs::read_to_string(path).map_err(|source| RunError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let yaml = serde_yaml::from_str::<serde_yaml::Value>(&content).map_err(|source| {
        RunError::InvalidValuesYaml {
            path: path.display().to_string(),
            source,
        }
    })?;
    serde_json::to_value(yaml).map_err(|source| RunError::InvalidValuesJson {
        path: path.display().to_string(),
        source,
    })
}
```

- [ ] **Step 4: Wire the CLI subcommand**

Modify `crates/chartwright-cli/src/main.rs` imports:

```rust
use chartwright_abi::RenderRequest;
use clap::{Parser, Subcommand};
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;
```

Add the subcommand variant:

```rust
Run {
    library_path: PathBuf,
    #[arg(long, default_value = "demo")]
    release_name: String,
    #[arg(long, default_value = "default")]
    namespace: String,
    #[arg(long)]
    values: Option<PathBuf>,
    #[arg(long, default_value = "1.30.0")]
    kube_version: String,
    #[arg(long = "api-version")]
    api_versions: Vec<String>,
},
```

Add match handling:

```rust
Command::Run {
    library_path,
    release_name,
    namespace,
    values,
    kube_version,
    api_versions,
} => {
    let values = match values {
        Some(path) => chartwright_cli::values_from_file(path),
        None => Ok(json!({})),
    };
    values.and_then(|values| {
        let rendered = chartwright_cli::run_chart_module(
            library_path,
            RenderRequest {
                release_name,
                namespace,
                values,
                kube_version,
                api_versions,
            },
        )?;
        std::io::stdout()
            .write_all(rendered.as_bytes())
            .map_err(|source| chartwright_cli::RunError::Io {
                path: "stdout".to_owned(),
                source,
            })?;
        Ok(())
    })
}
```

- [ ] **Step 5: Run focused tests**

Run:

```bash
cargo test -p chartwright-cli
```

Expected: PASS.

### Task 3: Final Verification

**Files:**
- Verify all modified files.

- [ ] **Step 1: Format**

Run:

```bash
cargo fmt
```

Expected: no output and exit 0.

- [ ] **Step 2: Full test suite**

Run:

```bash
cargo test
```

Expected: PASS.

- [ ] **Step 3: Review diff**

Run:

```bash
git diff --stat
git diff
```

Expected: changes are limited to the plan and CLI implementation.

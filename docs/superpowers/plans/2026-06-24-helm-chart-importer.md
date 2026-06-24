# Helm Chart Importer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust CLI that imports a local Helm chart into a generated loadable Rust module that renders Helm-template-style YAML.

**Architecture:** The workspace has three crates: `helm-rs-runtime` renders embedded chart files, `helm-rs-abi` defines the dynamic module ABI, and `helm-rs-cli` generates chart crates. Generated crates embed chart files and export ABI functions backed by the runtime.

**Tech Stack:** Rust 2021, Cargo workspace, `serde`, `serde_json`, `serde_yaml`, `thiserror`, `clap`, `tempfile`, `libloading`.

---

### Task 1: Workspace Skeleton And Fixture

**Files:**
- Create: `Cargo.toml`
- Create: `crates/helm-rs-runtime/Cargo.toml`
- Create: `crates/helm-rs-runtime/src/lib.rs`
- Create: `crates/helm-rs-abi/Cargo.toml`
- Create: `crates/helm-rs-abi/src/lib.rs`
- Create: `crates/helm-rs-cli/Cargo.toml`
- Create: `crates/helm-rs-cli/src/main.rs`
- Create: `fixtures/basic-chart/Chart.yaml`
- Create: `fixtures/basic-chart/values.yaml`
- Create: `fixtures/basic-chart/templates/configmap.yaml`
- Create: `fixtures/basic-chart/templates/_helpers.tpl`
- Create: `fixtures/basic-chart/golden.yaml`

- [ ] **Step 1: Add a workspace manifest with shared dependencies.**

Create `Cargo.toml`:

```toml
[workspace]
members = [
  "crates/helm-rs-abi",
  "crates/helm-rs-runtime",
  "crates/helm-rs-cli",
]
resolver = "2"

[workspace.package]
edition = "2021"
license = "MIT OR Apache-2.0"
repository = "https://example.invalid/helm-rs"

[workspace.dependencies]
clap = { version = "4.5", features = ["derive"] }
libloading = "0.8"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
tempfile = "3"
thiserror = "1"
```

- [ ] **Step 2: Add crate manifests and minimal library roots.**

Create minimal manifests for the three crates. `helm-rs-runtime` depends on `serde`, `serde_json`, `serde_yaml`, and `thiserror`. `helm-rs-abi` depends on `serde` and `serde_json`, with dev-dependencies `libloading` and `tempfile`. `helm-rs-cli` depends on `clap`, `helm-rs-runtime`, `serde_json`, `serde_yaml`, and `thiserror`, with dev-dependency `tempfile`.

- [ ] **Step 3: Add the fixture chart.**

Use chart name `basic-chart`, default `name: from-default`, and templates that exercise `.Release.Name`, `.Release.Namespace`, `.Chart.Name`, `.Values.name`, `include`, `quote`, and `nindent`.

- [ ] **Step 4: Run `cargo test` to confirm the empty workspace builds.**

Expected: the command compiles all crates and reports the default crate tests passing.

### Task 2: Runtime Chart Model And Rendering

**Files:**
- Modify: `crates/helm-rs-runtime/src/lib.rs`
- Create: `crates/helm-rs-runtime/tests/basic_render.rs`

- [ ] **Step 1: Write a failing render test.**

The test constructs a `Chart` with `Chart.yaml`, `values.yaml`, `_helpers.tpl`, and `configmap.yaml`, calls `render_chart`, and asserts the YAML equals `fixtures/basic-chart/golden.yaml`.

- [ ] **Step 2: Run the specific runtime test and confirm it fails because the API does not exist.**

Run: `cargo test -p helm-rs-runtime --test basic_render renders_basic_chart`

- [ ] **Step 3: Implement the runtime model.**

Add `ChartFile`, `Chart`, `RenderInput`, `ReleaseInput`, `CapabilitiesInput`, `RenderError`, and `render_chart`.

- [ ] **Step 4: Implement minimal template rendering.**

Support text templates with `{{ ... }}` actions, whitespace trimming markers, value paths, string literals, pipelines, `include`, `quote`, `nindent`, `default`, `upper`, `lower`, `trim`, `replace`, and `toYaml`. Unsupported actions return `RenderError::Unsupported`.

- [ ] **Step 5: Run the runtime test and confirm it passes.**

Run: `cargo test -p helm-rs-runtime --test basic_render`

### Task 3: ABI Contract

**Files:**
- Modify: `crates/helm-rs-abi/src/lib.rs`
- Create: `crates/helm-rs-abi/tests/buffer.rs`

- [ ] **Step 1: Write failing ABI buffer tests.**

Test that an owned buffer can be created from bytes, read as a slice, converted back to a boxed slice exactly once, and reports null misuse safely.

- [ ] **Step 2: Run the ABI tests and confirm they fail because the API does not exist.**

Run: `cargo test -p helm-rs-abi --test buffer`

- [ ] **Step 3: Implement `AbiBuffer`, `AbiResult`, `RenderRequest`, `ModuleInfo`, and helper allocation/free functions.**

Use `#[repr(C)]` for ABI structs. Keep JSON request and metadata structs serde-compatible.

- [ ] **Step 4: Run ABI tests and confirm they pass.**

Run: `cargo test -p helm-rs-abi --test buffer`

### Task 4: Importer CLI And Generated Crate

**Files:**
- Modify: `crates/helm-rs-cli/src/main.rs`
- Create: `crates/helm-rs-cli/tests/import.rs`

- [ ] **Step 1: Write a failing CLI import test.**

The test runs the CLI logic against `fixtures/basic-chart`, writes to a temp directory, and asserts generated `Cargo.toml` and `src/lib.rs` exist and contain `crate-type = ["cdylib", "rlib"]`, `helm_rs_render_json`, and embedded chart files.

- [ ] **Step 2: Run the CLI test and confirm it fails because importer logic does not exist.**

Run: `cargo test -p helm-rs-cli --test import`

- [ ] **Step 3: Implement importer logic.**

Parse `helm-rs import <chart-dir> --crate-dir <out-dir>`, validate `Chart.yaml`, collect files under `templates/`, and generate a crate with embedded chart files and ABI exports.

- [ ] **Step 4: Run CLI tests and confirm they pass.**

Run: `cargo test -p helm-rs-cli --test import`

### Task 5: End-To-End Dynamic Load

**Files:**
- Create: `tests/generated_module.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Write a failing integration test for generated module loading.**

The test imports `fixtures/basic-chart` into a temp crate, runs `cargo build --release` in that crate, loads the produced dynamic library with `libloading`, calls `helm_rs_render_json`, reads the returned buffer, frees it, and compares output to `fixtures/basic-chart/golden.yaml`.

- [ ] **Step 2: Run the integration test and confirm it fails before generated module support is complete.**

Run: `cargo test --test generated_module`

- [ ] **Step 3: Fix generated crate code until the dynamic load test passes.**

Ensure generated paths point to the local workspace crates using absolute path dependencies and export all required symbols.

- [ ] **Step 4: Run the end-to-end test and confirm it passes.**

Run: `cargo test --test generated_module`

### Task 6: Full Verification

**Files:**
- Modify only files needed by formatting or compiler feedback.

- [ ] **Step 1: Run formatting.**

Run: `cargo fmt --all`

- [ ] **Step 2: Run full tests.**

Run: `cargo test`

- [ ] **Step 3: Inspect git status.**

Run: `git status --short`

- [ ] **Step 4: Commit implementation.**

Commit all implementation files with message `Implement Helm chart importer MVP`.

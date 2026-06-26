# Helm Chart Importer Design

## Goal

Build a Rust tool that imports a local Helm chart and emits a Rust crate that can render Kubernetes manifests equivalent to `helm template` for that chart. The generated crate must build into a loadable dynamic library so another Rust program can load it at runtime, pass render inputs, and receive YAML output.

The first milestone targets template rendering only. It does not install resources, manage Helm releases, run hooks, contact a Kubernetes cluster, or implement upgrade/rollback behavior.

## Recommended Approach

Use a generated Rust module backed by a reusable runtime library.

The importer reads chart files, generates a crate that embeds those files, and wires the generated crate to a shared rendering runtime. The generated crate builds as a `cdylib` with a stable C ABI. Host programs can hot-load the compiled library with `libloading` or an equivalent dynamic loader.

This approach keeps generated artifacts deterministic, makes compatibility testable against `helm template`, and avoids a runtime dependency on the Helm CLI.

## Workspace Structure

The repository will be a Rust workspace with these crates:

- `chartwright-cli`: command-line importer.
- `chartwright-runtime`: chart model, values merging, template rendering, manifest assembly, and errors.
- `chartwright-abi`: shared ABI types and host-facing loading helpers.

Generated chart crates will depend on `chartwright-runtime` and `chartwright-abi`. They will expose C ABI functions for dynamic loading and can also expose a normal Rust API for static linking in tests.

## CLI Behavior

The initial importer command:

```text
chartwright import <chart-dir> --crate-dir <out-dir>
```

The command validates that `<chart-dir>` contains a Helm chart, reads the chart contents, and writes a generated Rust crate to `<out-dir>`.

The generated crate includes:

- `Cargo.toml` configured for `cdylib` output.
- Rust source that embeds `Chart.yaml`, `values.yaml`, template files, and helper templates.
- A small generated chart descriptor that passes embedded files to `chartwright-runtime`.
- ABI exports for host programs.

The importer should preserve enough source path metadata for errors to point back to the originating chart file.

## Dynamic Module Contract

The ABI boundary uses bytes rather than Rust-native types.

Render input is JSON:

```json
{
  "release_name": "my-release",
  "namespace": "default",
  "values": {},
  "kube_version": "1.30.0",
  "api_versions": ["apps/v1", "batch/v1"]
}
```

Render output is a UTF-8 YAML stream containing all rendered manifests.

The generated library exports:

- `chartwright_render_json(input_ptr, input_len, out_ptr)`: renders from JSON input and writes an owned output buffer handle into `out_ptr`.
- `chartwright_free(buffer)`: releases buffers allocated by the module. The host must free every successful render or metadata result with this function from the same loaded library.
- `chartwright_module_info()`: returns module metadata such as chart name, chart version, generated schema version, and runtime compatibility version through the same owned-buffer mechanism.

The exact ABI structs will be defined in `chartwright-abi` and versioned. Errors cross the ABI as structured JSON containing an error code, message, and optional file/line context.

## Runtime Rendering Semantics

The runtime renders manifests in the same broad mode as `helm template`:

1. Parse chart metadata and default values.
2. Merge chart defaults with caller-provided values, with caller values taking precedence.
3. Build Helm-like root objects: `.Values`, `.Chart`, `.Release`, and `.Capabilities`.
4. Load templates from `templates/`.
5. Register helper templates defined with `define`.
6. Render non-helper templates.
7. Drop empty manifests and Helm comments where appropriate.
8. Join manifests into one YAML stream.

Initial template support includes:

- value access through `.Values`, `.Chart`, `.Release`, and `.Capabilities`;
- `define`, `template`, and `include`;
- `if`, `with`, and `range`;
- pipelines;
- common functions: `default`, `quote`, `toYaml`, `indent`, `nindent`, `upper`, `lower`, `trim`, `dict`, `list`, `hasKey`, `required`, `printf`, `replace`, and basic boolean helpers.

Unsupported template features must produce explicit errors. The runtime must not silently emit manifests that it knows may be wrong.

## Out Of Scope For The First Milestone

- Applying resources to a cluster.
- Helm release storage, upgrades, rollbacks, and uninstall behavior.
- Hook lifecycle execution.
- `lookup` against a live cluster.
- Chart repository and OCI pulling.
- Dependency fetching or dependency build.
- Full Sprig function coverage.
- Exact byte-for-byte whitespace parity when the Kubernetes YAML documents are semantically equivalent.

## Error Handling

Errors should be structured and actionable.

The CLI reports chart import errors with file paths relative to the chart root. The runtime reports render errors with chart file context whenever possible. ABI errors return JSON so host programs can log or surface details without parsing human-only text.

Error categories:

- invalid chart structure;
- unsupported chart feature;
- unsupported template syntax or function;
- invalid render input JSON;
- template render failure;
- ABI misuse.

## Testing Strategy

Runtime tests use fixture charts and golden manifest outputs. The fixture set should start small and grow with each supported template construct.

Important test layers:

- unit tests for values merging, chart metadata parsing, root object construction, and individual functions;
- template renderer tests for conditionals, ranges, helpers, pipelines, indentation, and YAML conversion;
- CLI tests that import a fixture chart and inspect the generated crate layout;
- end-to-end tests that import a fixture chart, build the generated `cdylib`, load it with a host harness, render JSON input, and compare YAML output;
- optional Helm comparison tests that run `helm template` if `helm` is installed and skip otherwise.

Golden outputs are acceptable for deterministic fixture charts. Helm comparison tests provide confidence that the runtime remains compatible as support expands.

## Acceptance Criteria

The first complete implementation is acceptable when:

- `cargo test` passes for the workspace;
- `chartwright import fixtures/basic-chart --crate-dir <tmp>` creates a buildable generated crate;
- the generated crate builds as a dynamic library;
- a Rust host can hot-load the generated library, call the render ABI, and receive YAML;
- the basic fixture output matches the fixture golden output;
- unsupported template features return structured errors rather than partial or misleading manifests.

## Future Extensions

Later milestones can add broader Helm compatibility:

- more Sprig functions;
- chart dependencies;
- schema validation from `values.schema.json`;
- OCI and chart repository import;
- a richer safe Rust host API over the C ABI;
- release lifecycle behavior separate from the rendering-only generated module.

# Chart Module Run Subcommand Design

## Goal

Add a small wrapper tool for testing generated chart dynamic libraries. The wrapper will be a `chartwright` CLI subcommand that loads a compiled chart module, renders it with a test request, and prints the generated manifest to stdout.

## CLI Shape

Add:

```text
chartwright run <library-path> \
  --release-name <name> \
  --namespace <namespace> \
  [--values <json-or-yaml-file>] \
  [--kube-version <version>] \
  [--api-version <version>]...
```

Defaults:

- `--release-name`: `demo`
- `--namespace`: `default`
- `--kube-version`: `1.30.0`
- `--values`: omitted means an empty object
- `--api-version`: may be repeated and defaults to an empty list

The subcommand prints only the rendered manifest to stdout on success. Errors use the existing binary behavior: print `error: ...` to stderr and exit non-zero.

## Architecture

The CLI crate will depend on `chartwright-abi` with its default loader feature enabled. The implementation will reuse `chartwright_abi::LoadedChartModule` instead of calling dynamic symbols directly.

Add a library helper:

```rust
pub fn run_chart_module(
    library_path: impl AsRef<Path>,
    request: chartwright_abi::RenderRequest,
) -> Result<String, RunError>
```

`RunError` will wrap loader errors and values-file parsing errors. Keeping this in the library lets tests exercise the behavior without depending on process spawning.

## Data Flow

1. `chartwright run` parses command-line options.
2. If `--values` is set, the CLI reads the file and parses it as YAML, then converts it to `serde_json::Value`. If omitted, values are `{}`.
3. The CLI builds a `RenderRequest`.
4. `run_chart_module` loads the dynamic library with `LoadedChartModule::load`.
5. It calls `render` and returns the manifest.
6. The binary writes the manifest to stdout.

## Error Handling

- Missing or unreadable values files include the path in the error.
- Invalid values YAML includes the path and parse error.
- Dynamic library load errors come from `LoadedChartModule`.
- Module render errors remain structured inside `LoadError::Module` and are displayed through its existing error message.

## Tests

Add CLI crate tests that:

- Import `fixtures/basic-chart` into a temporary generated crate.
- Build the generated crate in release mode.
- Locate the platform-specific dynamic library in `target/release`.
- Call `run_chart_module` with a `RenderRequest`.
- Assert the output equals `fixtures/basic-chart/golden.yaml`.

This complements the existing root integration tests while covering the new public CLI helper.

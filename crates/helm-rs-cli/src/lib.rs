use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("chart is missing Chart.yaml at {0}")]
    MissingChartYaml(String),
    #[error("invalid Chart.yaml: {0}")]
    InvalidChartYaml(serde_yaml::Error),
    #[error("chart name is missing from Chart.yaml")]
    MissingChartName,
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

#[derive(Debug)]
struct SourceFile {
    path: String,
    content: String,
}

pub fn import_chart(
    chart_dir: impl AsRef<Path>,
    out_dir: impl AsRef<Path>,
) -> Result<(), ImportError> {
    let chart_dir = chart_dir.as_ref();
    let out_dir = out_dir.as_ref();
    let chart_yaml_path = chart_dir.join("Chart.yaml");
    if !chart_yaml_path.exists() {
        return Err(ImportError::MissingChartYaml(
            chart_yaml_path.display().to_string(),
        ));
    }

    let chart_yaml = read_to_string(&chart_yaml_path)?;
    let chart_doc: serde_yaml::Value =
        serde_yaml::from_str(&chart_yaml).map_err(ImportError::InvalidChartYaml)?;
    let chart_name = chart_doc
        .get("name")
        .and_then(serde_yaml::Value::as_str)
        .ok_or(ImportError::MissingChartName)?
        .to_owned();
    let chart_version = chart_doc
        .get("version")
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or("0.0.0")
        .to_owned();

    let mut files = vec![SourceFile {
        path: "Chart.yaml".to_owned(),
        content: chart_yaml,
    }];
    let values_path = chart_dir.join("values.yaml");
    if values_path.exists() {
        files.push(SourceFile {
            path: "values.yaml".to_owned(),
            content: read_to_string(&values_path)?,
        });
    }
    collect_template_files(chart_dir, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));

    fs::create_dir_all(out_dir.join("src")).map_err(|source| ImportError::Io {
        path: out_dir.join("src").display().to_string(),
        source,
    })?;

    let root = workspace_root();
    write_file(
        &out_dir.join("Cargo.toml"),
        &generated_manifest(&chart_name, &root),
    )?;
    write_file(
        &out_dir.join("src/lib.rs"),
        &generated_lib_rs(&chart_name, &chart_version, &files),
    )?;
    Ok(())
}

fn collect_template_files(
    chart_dir: &Path,
    files: &mut Vec<SourceFile>,
) -> Result<(), ImportError> {
    let templates_dir = chart_dir.join("templates");
    if !templates_dir.exists() {
        return Ok(());
    }
    collect_dir(chart_dir, &templates_dir, files)
}

fn collect_dir(
    chart_dir: &Path,
    dir: &Path,
    files: &mut Vec<SourceFile>,
) -> Result<(), ImportError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|source| ImportError::Io {
        path: dir.display().to_string(),
        source,
    })? {
        entries.push(entry.map_err(|source| ImportError::Io {
            path: dir.display().to_string(),
            source,
        })?);
    }
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_dir(chart_dir, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(chart_dir)
                .expect("template file is under chart root")
                .to_string_lossy()
                .replace('\\', "/");
            files.push(SourceFile {
                path: relative,
                content: read_to_string(&path)?,
            });
        }
    }
    Ok(())
}

fn generated_manifest(chart_name: &str, root: &Path) -> String {
    let package_name = sanitize_crate_name(chart_name);
    format!(
        r#"[package]
name = "{package_name}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
helm-rs-abi = {{ path = "{}" }}
helm-rs-runtime = {{ path = "{}" }}
serde_json = "1"
"#,
        root.join("crates/helm-rs-abi").display(),
        root.join("crates/helm-rs-runtime").display()
    )
}

fn generated_lib_rs(chart_name: &str, chart_version: &str, files: &[SourceFile]) -> String {
    let file_entries = files
        .iter()
        .map(|file| {
            format!(
                "        ChartFile::new({:?}, {:?}),\n",
                file.path, file.content
            )
        })
        .collect::<String>();
    format!(
        r#"use helm_rs_abi::{{buffer_from_bytes, error_buffer, free_buffer, AbiBuffer, ModuleInfo, RenderRequest, ABI_VERSION}};
use helm_rs_runtime::{{render_chart, CapabilitiesInput, Chart, ChartFile, ReleaseInput, RenderInput}};

fn chart() -> Chart {{
    Chart {{
        files: vec![
{file_entries}        ],
    }}
}}

fn render(input: RenderRequest) -> Result<String, String> {{
    let input = RenderInput {{
        release: ReleaseInput {{
            name: input.release_name,
            namespace: input.namespace,
        }},
        values: input.values,
        capabilities: CapabilitiesInput {{
            kube_version: input.kube_version,
            api_versions: input.api_versions,
        }},
    }};
    render_chart(&chart(), &input).map_err(|error| error.to_string())
}}

#[no_mangle]
pub unsafe extern "C" fn helm_rs_render_json(input_ptr: *const u8, input_len: usize, out_ptr: *mut AbiBuffer) -> i32 {{
    if input_ptr.is_null() || out_ptr.is_null() {{
        return 2;
    }}
    let input = std::slice::from_raw_parts(input_ptr, input_len);
    let request = match serde_json::from_slice::<RenderRequest>(input) {{
        Ok(request) => request,
        Err(error) => {{
            *out_ptr = error_buffer("invalid_input", error.to_string());
            return 1;
        }}
    }};
    match render(request) {{
        Ok(output) => {{
            *out_ptr = buffer_from_bytes(output.as_bytes());
            0
        }}
        Err(error) => {{
            *out_ptr = error_buffer("render_error", error);
            1
        }}
    }}
}}

#[no_mangle]
pub unsafe extern "C" fn helm_rs_module_info(out_ptr: *mut AbiBuffer) -> i32 {{
    if out_ptr.is_null() {{
        return 2;
    }}
    let info = ModuleInfo {{
        abi_version: ABI_VERSION,
        chart_name: {chart_name:?}.to_owned(),
        chart_version: {chart_version:?}.to_owned(),
        runtime_version: helm_rs_runtime::runtime_version().to_owned(),
    }};
    match serde_json::to_vec(&info) {{
        Ok(output) => {{
            *out_ptr = buffer_from_bytes(&output);
            0
        }}
        Err(error) => {{
            *out_ptr = buffer_from_bytes(error.to_string().as_bytes());
            1
        }}
    }}
}}

#[no_mangle]
pub unsafe extern "C" fn helm_rs_free(buffer: AbiBuffer) {{
    free_buffer(buffer);
}}
"#
    )
}

fn sanitize_crate_name(name: &str) -> String {
    let mut sanitized = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
        } else {
            sanitized.push('_');
        }
    }
    if sanitized
        .chars()
        .next()
        .map(|ch| ch.is_ascii_digit())
        .unwrap_or(true)
    {
        sanitized.insert_str(0, "chart_");
    }
    sanitized
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("helm-rs-cli is under crates/helm-rs-cli")
        .to_owned()
}

fn read_to_string(path: &Path) -> Result<String, ImportError> {
    fs::read_to_string(path).map_err(|source| ImportError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn write_file(path: &Path, content: &str) -> Result<(), ImportError> {
    fs::write(path, content).map_err(|source| ImportError::Io {
        path: path.display().to_string(),
        source,
    })
}

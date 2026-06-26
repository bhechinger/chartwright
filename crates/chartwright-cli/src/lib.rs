use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chartwright_abi::{LoadedChartModule, RenderRequest};
pub use chartwright_events::{
    Event, EventLevel, EventSink, InMemoryEventSink, NoopEventSink, StderrEventSink,
};
use thiserror::Error;

#[derive(Clone)]
struct EventProducer<S> {
    sink: S,
    id: String,
    label: String,
    started: Instant,
}

impl<S: EventSink> EventProducer<S> {
    fn start(sink: S, id: impl Into<String>, label: impl Into<String>, detail: String) -> Self {
        let id = id.into();
        let label = label.into();
        sink.emit(Event::StepStarted {
            id: id.clone(),
            label: label.clone(),
            detail: Some(detail),
        });
        Self {
            sink,
            id,
            label,
            started: Instant::now(),
        }
    }

    fn detail(&self, detail: impl Into<String>) {
        self.sink.emit(Event::StepDetail {
            id: self.id.clone(),
            detail: detail.into(),
        });
    }

    fn log(&self, level: EventLevel, message: impl Into<String>) {
        self.sink.emit(Event::Log {
            level,
            message: message.into(),
        });
    }

    fn finish(&self, message: impl Into<String>) {
        self.sink.emit(Event::StepFinished {
            id: self.id.clone(),
            message: message.into(),
            elapsed: self.started.elapsed(),
        });
    }

    fn fail(&self, message: impl Into<String>) {
        let message = message.into();
        self.sink.emit(Event::StepFailed {
            id: self.id.clone(),
            message: format!("{}: {message}", self.label),
        });
        self.log(EventLevel::Error, message);
    }
}

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

#[derive(Debug)]
struct SourceFile {
    path: String,
    content: String,
}

pub fn run_chart_module(
    library_path: impl AsRef<Path>,
    request: RenderRequest,
) -> Result<String, RunError> {
    let module = LoadedChartModule::load(library_path)?;
    module.render(request).map_err(RunError::from)
}

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

pub fn import_chart(
    chart_dir: impl AsRef<Path>,
    out_dir: impl AsRef<Path>,
) -> Result<(), ImportError> {
    import_chart_with_events(chart_dir, out_dir, NoopEventSink)
}

pub fn import_chart_with_events<S: EventSink>(
    chart_dir: impl AsRef<Path>,
    out_dir: impl AsRef<Path>,
    events: S,
) -> Result<(), ImportError> {
    let chart_dir = chart_dir.as_ref();
    let out_dir = out_dir.as_ref();
    let progress = EventProducer::start(
        events,
        "import-chart",
        "import chart",
        format!("{} -> {}", chart_dir.display(), out_dir.display()),
    );
    let result = import_chart_inner(chart_dir, out_dir, &progress);
    match &result {
        Ok(()) => progress.finish(format!("generated chart crate at {}", out_dir.display())),
        Err(error) => progress.fail(error.to_string()),
    }
    result
}

fn import_chart_inner<S: EventSink>(
    chart_dir: &Path,
    out_dir: &Path,
    progress: &EventProducer<S>,
) -> Result<(), ImportError> {
    let chart_yaml_path = chart_dir.join("Chart.yaml");
    progress.detail(format!("checking {}", chart_yaml_path.display()));
    if !chart_yaml_path.exists() {
        return Err(ImportError::MissingChartYaml(
            chart_yaml_path.display().to_string(),
        ));
    }

    progress.detail(format!("reading {}", chart_yaml_path.display()));
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
    progress.detail(format!("parsed chart {chart_name} {chart_version}"));

    let mut files = vec![SourceFile {
        path: "Chart.yaml".to_owned(),
        content: chart_yaml,
    }];
    let values_path = chart_dir.join("values.yaml");
    if values_path.exists() {
        progress.detail(format!("reading {}", values_path.display()));
        files.push(SourceFile {
            path: "values.yaml".to_owned(),
            content: read_to_string(&values_path)?,
        });
    } else {
        progress.log(
            EventLevel::Warn,
            format!("values.yaml not found under {}", chart_dir.display()),
        );
    }
    collect_template_files(chart_dir, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    progress.detail(format!("collected {} chart files", files.len()));

    progress.detail(format!("creating {}", out_dir.join("src").display()));
    fs::create_dir_all(out_dir.join("src")).map_err(|source| ImportError::Io {
        path: out_dir.join("src").display().to_string(),
        source,
    })?;

    progress.detail(format!("writing {}", out_dir.join("Cargo.toml").display()));
    write_file(
        &out_dir.join("Cargo.toml"),
        &generated_manifest(&chart_name, out_dir, &workspace_root()),
    )?;
    progress.detail(format!("writing {}", out_dir.join("src/lib.rs").display()));
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

fn generated_manifest(chart_name: &str, out_dir: &Path, root: &Path) -> String {
    let package_name = sanitize_crate_name(chart_name);
    let abi_path = relative_path(out_dir, &root.join("crates/chartwright-abi"));
    let runtime_path = relative_path(out_dir, &root.join("crates/chartwright-runtime"));
    format!(
        r#"[package]
name = "{package_name}"
version = "0.1.0"
edition = "2021"

[workspace]

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
chartwright-abi = {{ path = "{}", default-features = false }}
chartwright-runtime = {{ path = "{}" }}
serde_json = "1"
"#,
        abi_path.display(),
        runtime_path.display()
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
        r#"use chartwright_abi::{{buffer_from_bytes, error_buffer, free_buffer, AbiBuffer, ModuleInfo, RenderRequest, ABI_VERSION}};
use chartwright_runtime::{{render_chart, CapabilitiesInput, Chart, ChartFile, ReleaseInput, RenderInput}};

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
pub unsafe extern "C" fn chartwright_render_json(input_ptr: *const u8, input_len: usize, out_ptr: *mut AbiBuffer) -> i32 {{
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
pub unsafe extern "C" fn chartwright_module_info(out_ptr: *mut AbiBuffer) -> i32 {{
    if out_ptr.is_null() {{
        return 2;
    }}
    let info = ModuleInfo {{
        abi_version: ABI_VERSION,
        chart_name: {chart_name:?}.to_owned(),
        chart_version: {chart_version:?}.to_owned(),
        runtime_version: chartwright_runtime::runtime_version().to_owned(),
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
pub unsafe extern "C" fn chartwright_free(buffer: AbiBuffer) {{
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
        .expect("chartwright-cli is under crates/chartwright-cli")
        .to_owned()
}

fn relative_path(from_dir: &Path, to: &Path) -> PathBuf {
    let from = absolute_path(from_dir);
    let to = absolute_path(to);
    let from_components: Vec<_> = from.components().collect();
    let to_components: Vec<_> = to.components().collect();
    let common_len = from_components
        .iter()
        .zip(&to_components)
        .take_while(|(left, right)| left == right)
        .count();

    if common_len == 0 {
        return to;
    }

    let mut relative = PathBuf::new();
    for _ in common_len..from_components.len() {
        relative.push("..");
    }
    for component in &to_components[common_len..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        relative
    }
}

fn absolute_path(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .expect("current directory is available")
            .join(path)
    }
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

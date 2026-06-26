use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use thiserror::Error;

pub fn runtime_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartFile {
    pub path: String,
    pub content: String,
}

impl ChartFile {
    pub fn new(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chart {
    pub files: Vec<ChartFile>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderInput {
    pub release: ReleaseInput,
    #[serde(default)]
    pub values: Value,
    pub capabilities: CapabilitiesInput,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInput {
    pub name: String,
    pub namespace: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitiesInput {
    pub kube_version: String,
    #[serde(default)]
    pub api_versions: Vec<String>,
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("chart is missing {0}")]
    MissingFile(&'static str),
    #[error("invalid yaml in {path}: {source}")]
    InvalidYaml {
        path: String,
        source: serde_yaml::Error,
    },
    #[error("unsupported template action in {path}: {action}")]
    Unsupported { path: String, action: String },
    #[error("template render error in {path}: {message}")]
    Template { path: String, message: String },
}

#[derive(Debug, Deserialize)]
struct ChartMetadata {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(rename = "appVersion", default)]
    app_version: String,
}

pub fn render_chart(chart: &Chart, input: &RenderInput) -> Result<String, RenderError> {
    let chart_yaml = find_file(chart, "Chart.yaml")?;
    let metadata: ChartMetadata =
        serde_yaml::from_str(&chart_yaml.content).map_err(|source| RenderError::InvalidYaml {
            path: chart_yaml.path.clone(),
            source,
        })?;
    let default_values = match chart.files.iter().find(|file| file.path == "values.yaml") {
        Some(file) => yaml_to_json(&file.path, &file.content)?,
        None => json!({}),
    };
    let values = merge_values(default_values, input.values.clone());
    let root = build_root(&metadata, input, values);
    let helpers = collect_helpers(chart)?;
    let mut rendered = Vec::new();

    let mut templates: Vec<&ChartFile> = chart
        .files
        .iter()
        .filter(|file| file.path.starts_with("templates/"))
        .filter(|file| !template_name(file).starts_with('_'))
        .collect();
    templates.sort_by(|left, right| left.path.cmp(&right.path));

    for file in templates {
        let manifest = render_template(&file.content, &file.path, &root, &helpers)?;
        let trimmed = manifest.trim();
        if !trimmed.is_empty() {
            rendered.push(trimmed.to_owned());
        }
    }

    if rendered.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("---\n{}\n", rendered.join("\n---\n")))
    }
}

fn find_file<'a>(chart: &'a Chart, path: &'static str) -> Result<&'a ChartFile, RenderError> {
    chart
        .files
        .iter()
        .find(|file| file.path == path)
        .ok_or(RenderError::MissingFile(path))
}

fn yaml_to_json(path: &str, content: &str) -> Result<Value, RenderError> {
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|source| RenderError::InvalidYaml {
            path: path.to_owned(),
            source,
        })?;
    serde_json::to_value(yaml).map_err(|source| RenderError::Template {
        path: path.to_owned(),
        message: source.to_string(),
    })
}

fn merge_values(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                let merged = match base.remove(&key) {
                    Some(existing) => merge_values(existing, value),
                    None => value,
                };
                base.insert(key, merged);
            }
            Value::Object(base)
        }
        (_, Value::Null) => Value::Null,
        (_, overlay) => overlay,
    }
}

fn build_root(metadata: &ChartMetadata, input: &RenderInput, values: Value) -> Value {
    json!({
        "Values": values,
        "Chart": {
            "Name": metadata.name,
            "Version": metadata.version,
            "AppVersion": metadata.app_version,
        },
        "Release": {
            "Name": input.release.name,
            "Namespace": input.release.namespace,
            "Service": "Helm",
            "IsInstall": true,
            "IsUpgrade": false,
        },
        "Capabilities": {
            "KubeVersion": {
                "Version": input.capabilities.kube_version,
            },
            "APIVersions": input.capabilities.api_versions,
        },
    })
}

fn template_name(file: &ChartFile) -> &str {
    file.path.rsplit('/').next().unwrap_or(&file.path)
}

fn collect_helpers(chart: &Chart) -> Result<BTreeMap<String, String>, RenderError> {
    let mut helpers = BTreeMap::new();
    for file in chart
        .files
        .iter()
        .filter(|file| file.path.starts_with("templates/"))
        .filter(|file| template_name(file).starts_with('_'))
    {
        let mut rest = file.content.as_str();
        while let Some(start) = rest.find("define") {
            let before = &rest[..start];
            before.rfind("{{").ok_or_else(|| RenderError::Template {
                path: file.path.clone(),
                message: "define outside template action".to_owned(),
            })?;
            let after_define = &rest[start + "define".len()..];
            let name_start = after_define
                .find('"')
                .ok_or_else(|| RenderError::Template {
                    path: file.path.clone(),
                    message: "helper define is missing name".to_owned(),
                })?;
            let after_quote = &after_define[name_start + 1..];
            let name_end = after_quote.find('"').ok_or_else(|| RenderError::Template {
                path: file.path.clone(),
                message: "helper define has unterminated name".to_owned(),
            })?;
            let name = &after_quote[..name_end];
            let body_start = start
                + "define".len()
                + name_start
                + 1
                + name_end
                + 1
                + after_quote[name_end + 1..]
                    .find("}}")
                    .ok_or_else(|| RenderError::Template {
                        path: file.path.clone(),
                        message: "helper define action is unterminated".to_owned(),
                    })?
                + 2;
            let (end, end_len) =
                find_helper_end(&rest[body_start..]).ok_or_else(|| RenderError::Template {
                    path: file.path.clone(),
                    message: "helper define is missing end".to_owned(),
                })?;
            let body = rest[body_start..body_start + end].trim().to_owned();
            helpers.insert(name.to_owned(), body);
            rest = &rest[body_start + end + end_len..];
        }
    }
    Ok(helpers)
}

fn find_helper_end(input: &str) -> Option<(usize, usize)> {
    [
        "{{- end -}}",
        "{{- end}}",
        "{{end -}}",
        "{{end}}",
        "{{ end }}",
    ]
    .iter()
    .filter_map(|marker| input.find(marker).map(|index| (index, marker.len())))
    .min_by_key(|(index, _)| *index)
}

fn render_template(
    template: &str,
    path: &str,
    root: &Value,
    helpers: &BTreeMap<String, String>,
) -> Result<String, RenderError> {
    let mut output = String::new();
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        let end = after_open.find("}}").ok_or_else(|| RenderError::Template {
            path: path.to_owned(),
            message: "unterminated template action".to_owned(),
        })?;
        let raw_action = &after_open[..end];
        let trim_left = raw_action.starts_with('-');
        let trim_right = raw_action.ends_with('-');
        if trim_left {
            trim_trailing_whitespace(&mut output);
        }
        let action = raw_action
            .trim_start_matches('-')
            .trim_end_matches('-')
            .trim();
        if !(action.starts_with("/*") && action.ends_with("*/")) {
            let value = eval_pipeline(action, path, root, helpers)?;
            output.push_str(&value_to_output(&value));
        }
        rest = &after_open[end + 2..];
        if trim_right {
            rest = rest.trim_start_matches(char::is_whitespace);
        }
    }

    output.push_str(rest);
    Ok(output)
}

fn trim_trailing_whitespace(output: &mut String) {
    while output.ends_with(char::is_whitespace) {
        output.pop();
    }
}

fn eval_pipeline(
    action: &str,
    path: &str,
    root: &Value,
    helpers: &BTreeMap<String, String>,
) -> Result<Value, RenderError> {
    let mut parts = split_pipeline(action);
    let first = parts.remove(0);
    let mut value = eval_command(&first, None, path, root, helpers)?;
    for part in parts {
        value = eval_command(&part, Some(value), path, root, helpers)?;
    }
    Ok(value)
}

fn split_pipeline(action: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    for ch in action.chars() {
        match ch {
            '"' => {
                in_string = !in_string;
                current.push(ch);
            }
            '|' if !in_string => {
                parts.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    parts.push(current.trim().to_owned());
    parts
}

fn eval_command(
    command: &str,
    piped: Option<Value>,
    path: &str,
    root: &Value,
    helpers: &BTreeMap<String, String>,
) -> Result<Value, RenderError> {
    let tokens = split_tokens(command);
    let Some(first) = tokens.first() else {
        return Ok(Value::String(String::new()));
    };
    match (first.as_str(), piped) {
        ("include", None) => {
            let name = tokens
                .get(1)
                .and_then(|token| unquote(token))
                .ok_or_else(|| RenderError::Unsupported {
                    path: path.to_owned(),
                    action: command.to_owned(),
                })?;
            let helper = helpers
                .get(name)
                .ok_or_else(|| RenderError::Template {
                    path: path.to_owned(),
                    message: format!("unknown helper template {name}"),
                })?
                .clone();
            Ok(Value::String(render_template(
                &helper, path, root, helpers,
            )?))
        }
        ("quote", Some(value)) => Ok(Value::String(format!("\"{}\"", scalar_to_string(&value)))),
        ("nindent", Some(value)) => {
            let width = parse_width(tokens.get(1), command, path)?;
            Ok(Value::String(nindent(width, &scalar_to_string(&value))))
        }
        ("indent", Some(value)) => {
            let width = parse_width(tokens.get(1), command, path)?;
            Ok(Value::String(indent(width, &scalar_to_string(&value))))
        }
        ("toYaml", None) => {
            let target = tokens.get(1).ok_or_else(|| RenderError::Unsupported {
                path: path.to_owned(),
                action: command.to_owned(),
            })?;
            let value = eval_path(target, root, path)?;
            let yaml = serde_yaml::to_string(&value).map_err(|source| RenderError::Template {
                path: path.to_owned(),
                message: source.to_string(),
            })?;
            Ok(Value::String(yaml.trim_end().to_owned()))
        }
        ("default", Some(value)) => {
            if is_empty_value(&value) {
                let fallback = tokens.get(1).ok_or_else(|| RenderError::Unsupported {
                    path: path.to_owned(),
                    action: command.to_owned(),
                })?;
                eval_literal_or_path(fallback, root, path)
            } else {
                Ok(value)
            }
        }
        ("upper", Some(value)) => Ok(Value::String(scalar_to_string(&value).to_uppercase())),
        ("lower", Some(value)) => Ok(Value::String(scalar_to_string(&value).to_lowercase())),
        ("trim", Some(value)) => Ok(Value::String(scalar_to_string(&value).trim().to_owned())),
        ("replace", Some(value)) => {
            let from = tokens
                .get(1)
                .and_then(|token| unquote(token))
                .ok_or_else(|| RenderError::Unsupported {
                    path: path.to_owned(),
                    action: command.to_owned(),
                })?;
            let to = tokens
                .get(2)
                .and_then(|token| unquote(token))
                .ok_or_else(|| RenderError::Unsupported {
                    path: path.to_owned(),
                    action: command.to_owned(),
                })?;
            Ok(Value::String(scalar_to_string(&value).replace(from, to)))
        }
        (_, None) => eval_literal_or_path(command, root, path),
        (_, Some(_)) => Err(RenderError::Unsupported {
            path: path.to_owned(),
            action: command.to_owned(),
        }),
    }
}

fn split_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    for ch in command.chars() {
        match ch {
            '"' => {
                in_string = !in_string;
                current.push(ch);
            }
            ch if ch.is_whitespace() && !in_string => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn eval_literal_or_path(token: &str, root: &Value, path: &str) -> Result<Value, RenderError> {
    if let Some(value) = unquote(token) {
        Ok(Value::String(value.to_owned()))
    } else if token.starts_with('.') {
        eval_path(token, root, path)
    } else {
        Err(RenderError::Unsupported {
            path: path.to_owned(),
            action: token.to_owned(),
        })
    }
}

fn eval_path(token: &str, root: &Value, path: &str) -> Result<Value, RenderError> {
    let mut current = root;
    for segment in token.trim_start_matches('.').split('.') {
        if segment.is_empty() {
            continue;
        }
        current = current.get(segment).ok_or_else(|| RenderError::Template {
            path: path.to_owned(),
            message: format!("unknown value path {token}"),
        })?;
    }
    Ok(current.clone())
}

fn unquote(token: &str) -> Option<&str> {
    token.strip_prefix('"')?.strip_suffix('"')
}

fn parse_width(token: Option<&String>, command: &str, path: &str) -> Result<usize, RenderError> {
    token
        .and_then(|token| token.parse().ok())
        .ok_or_else(|| RenderError::Unsupported {
            path: path.to_owned(),
            action: command.to_owned(),
        })
}

fn value_to_output(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => scalar_to_string(value),
    }
}

fn scalar_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(false) => true,
        Value::String(value) => value.is_empty(),
        Value::Array(value) => value.is_empty(),
        Value::Object(value) => value.is_empty(),
        Value::Number(_) | Value::Bool(true) => false,
    }
}

fn indent(width: usize, value: &str) -> String {
    let padding = " ".repeat(width);
    value
        .lines()
        .map(|line| format!("{padding}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn nindent(width: usize, value: &str) -> String {
    format!("\n{}", indent(width, value))
}

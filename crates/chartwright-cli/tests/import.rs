use std::process::Command;

use chartwright_abi::RenderRequest;
use chartwright_cli::{
    import_chart, import_chart_with_events, Event, EventLevel, InMemoryEventSink,
};

#[test]
fn imports_basic_chart_to_generated_crate() {
    std::fs::create_dir_all("../../target").unwrap();
    let temp = tempfile::tempdir_in("../../target").unwrap();
    let out_dir = temp.path().join("generated-basic-chart");

    import_chart("../../fixtures/basic-chart", &out_dir).unwrap();

    let cargo_toml = std::fs::read_to_string(out_dir.join("Cargo.toml")).unwrap();
    let lib_rs = std::fs::read_to_string(out_dir.join("src/lib.rs")).unwrap();

    assert!(cargo_toml.contains("crate-type = [\"cdylib\", \"rlib\"]"));
    assert!(cargo_toml.contains("[workspace]"));
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .display()
        .to_string();
    assert!(!cargo_toml.contains(&workspace_root));
    assert!(lib_rs.contains("chartwright_render_json"));
    assert!(lib_rs.contains("Chart.yaml"));
    assert!(lib_rs.contains("templates/configmap.yaml"));
    assert!(lib_rs.contains("templates/_helpers.tpl"));
}

#[test]
fn import_emits_detailed_status_events() {
    let temp = tempfile::tempdir().unwrap();
    let out_dir = temp.path().join("generated-basic-chart");
    let events = InMemoryEventSink::default();

    import_chart_with_events("../../fixtures/basic-chart", &out_dir, events.clone()).unwrap();

    let emitted = events.events();
    assert!(matches!(
        &emitted[0],
        Event::StepStarted { label, detail: Some(detail), .. }
            if label == "import chart" && detail.contains("fixtures/basic-chart")
    ));
    assert!(emitted.iter().any(|event| matches!(
        event,
        Event::StepDetail { detail, .. } if detail == "parsed chart basic-chart 0.1.0"
    )));
    assert!(emitted.iter().any(|event| matches!(
        event,
        Event::StepDetail { detail, .. } if detail == "collected 4 chart files"
    )));
    assert!(emitted.iter().any(|event| matches!(
        event,
        Event::StepFinished { message, .. } if message.contains("generated chart crate")
    )));
}

#[test]
fn import_emits_error_event_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    let missing_chart = temp.path().join("missing-chart");
    let out_dir = temp.path().join("generated-missing-chart");
    let events = InMemoryEventSink::default();

    let err = import_chart_with_events(&missing_chart, &out_dir, events.clone()).unwrap_err();

    assert!(err.to_string().contains("Chart.yaml"));
    assert!(events.events().iter().any(|event| matches!(
        event,
        Event::Log { level: EventLevel::Error, message }
            if message.contains("chart is missing Chart.yaml")
    )));
}

#[test]
#[ignore = "builds a generated cdylib with cargo build --release"]
fn run_chart_module_renders_generated_library() {
    std::fs::create_dir_all("../../target").unwrap();
    let temp = tempfile::tempdir_in("../../target").unwrap();
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
    assert_eq!(
        values["image"]["repository"],
        serde_json::json!("example/app")
    );
}

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

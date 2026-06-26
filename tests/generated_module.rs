use std::process::Command;

use chartwright_abi::{AbiBuffer, LoadError, LoadedChartModule, RenderRequest};
use libloading::{Library, Symbol};

type RenderJson = unsafe extern "C" fn(*const u8, usize, *mut AbiBuffer) -> i32;
type FreeBuffer = unsafe extern "C" fn(AbiBuffer);

#[test]
fn generated_module_renders_when_hot_loaded() {
    let temp = tempfile::tempdir().unwrap();
    let generated = temp.path().join("generated-basic-chart");
    chartwright_cli::import_chart("fixtures/basic-chart", &generated).unwrap();

    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(&generated)
        .status()
        .unwrap();
    assert!(status.success());

    let library_path = dynamic_library_path(&generated, "basic_chart");
    let request = RenderRequest {
        release_name: "demo".to_owned(),
        namespace: "testing".to_owned(),
        values: serde_json::json!({}),
        kube_version: "1.30.0".to_owned(),
        api_versions: vec!["v1".to_owned()],
    };
    let request_json = serde_json::to_vec(&request).unwrap();
    let expected = std::fs::read_to_string("fixtures/basic-chart/golden.yaml").unwrap();

    unsafe {
        let library = Library::new(library_path).unwrap();
        let render: Symbol<RenderJson> = library.get(b"chartwright_render_json").unwrap();
        let free: Symbol<FreeBuffer> = library.get(b"chartwright_free").unwrap();
        let mut output = AbiBuffer::empty();

        let code = render(request_json.as_ptr(), request_json.len(), &mut output);
        let rendered = String::from_utf8(output.as_slice().to_vec()).unwrap();
        free(output);

        assert_eq!(code, 0, "{rendered}");
        assert_eq!(rendered, expected);
    }
}

#[test]
fn generated_module_renders_through_safe_loader() {
    let temp = tempfile::tempdir().unwrap();
    let generated = temp.path().join("generated-basic-chart");
    chartwright_cli::import_chart("fixtures/basic-chart", &generated).unwrap();

    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(&generated)
        .status()
        .unwrap();
    assert!(status.success());

    let module = LoadedChartModule::load(dynamic_library_path(&generated, "basic_chart")).unwrap();
    let info = module.info().unwrap();
    let rendered = module
        .render(RenderRequest {
            release_name: "demo".to_owned(),
            namespace: "testing".to_owned(),
            values: serde_json::json!({}),
            kube_version: "1.30.0".to_owned(),
            api_versions: vec!["v1".to_owned()],
        })
        .unwrap();

    assert_eq!(info.chart_name, "basic-chart");
    assert_eq!(
        rendered,
        std::fs::read_to_string("fixtures/basic-chart/golden.yaml").unwrap()
    );
}

#[test]
fn generated_module_returns_structured_json_errors() {
    let temp = tempfile::tempdir().unwrap();
    let chart = temp.path().join("unsupported-chart");
    std::fs::create_dir_all(chart.join("templates")).unwrap();
    std::fs::write(
        chart.join("Chart.yaml"),
        "apiVersion: v2\nname: unsupported-chart\nversion: 0.1.0\n",
    )
    .unwrap();
    std::fs::write(chart.join("values.yaml"), "{}\n").unwrap();
    std::fs::write(
        chart.join("templates/configmap.yaml"),
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {{ lookup \"v1\" \"ConfigMap\" \"\" \"\" }}\n",
    )
    .unwrap();

    let generated = temp.path().join("generated-unsupported-chart");
    chartwright_cli::import_chart(&chart, &generated).unwrap();
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(&generated)
        .status()
        .unwrap();
    assert!(status.success());

    let request = RenderRequest {
        release_name: "demo".to_owned(),
        namespace: "testing".to_owned(),
        values: serde_json::json!({}),
        kube_version: "1.30.0".to_owned(),
        api_versions: vec!["v1".to_owned()],
    };
    let request_json = serde_json::to_vec(&request).unwrap();

    unsafe {
        let library = Library::new(dynamic_library_path(&generated, "unsupported_chart")).unwrap();
        let render: Symbol<RenderJson> = library.get(b"chartwright_render_json").unwrap();
        let free: Symbol<FreeBuffer> = library.get(b"chartwright_free").unwrap();
        let mut output = AbiBuffer::empty();

        let code = render(request_json.as_ptr(), request_json.len(), &mut output);
        let rendered = String::from_utf8(output.as_slice().to_vec()).unwrap();
        free(output);
        let error: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(code, 1);
        assert_eq!(error["code"], "render_error");
        assert!(error["message"].as_str().unwrap().contains("unsupported"));
    }
}

#[test]
fn safe_loader_returns_module_errors() {
    let temp = tempfile::tempdir().unwrap();
    let chart = temp.path().join("unsupported-chart");
    std::fs::create_dir_all(chart.join("templates")).unwrap();
    std::fs::write(
        chart.join("Chart.yaml"),
        "apiVersion: v2\nname: unsupported-chart\nversion: 0.1.0\n",
    )
    .unwrap();
    std::fs::write(chart.join("values.yaml"), "{}\n").unwrap();
    std::fs::write(
        chart.join("templates/configmap.yaml"),
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {{ lookup \"v1\" \"ConfigMap\" \"\" \"\" }}\n",
    )
    .unwrap();

    let generated = temp.path().join("generated-unsupported-chart");
    chartwright_cli::import_chart(&chart, &generated).unwrap();
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(&generated)
        .status()
        .unwrap();
    assert!(status.success());

    let module =
        LoadedChartModule::load(dynamic_library_path(&generated, "unsupported_chart")).unwrap();
    let error = module
        .render(RenderRequest {
            release_name: "demo".to_owned(),
            namespace: "testing".to_owned(),
            values: serde_json::json!({}),
            kube_version: "1.30.0".to_owned(),
            api_versions: vec!["v1".to_owned()],
        })
        .unwrap_err();

    match error {
        LoadError::Module { code, message } => {
            assert_eq!(code, "render_error");
            assert!(message.contains("unsupported"));
        }
        other => panic!("expected module error, got {other:?}"),
    }
}

#[test]
fn safe_loader_rejects_abi_version_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let generated = temp.path().join("generated-basic-chart");
    chartwright_cli::import_chart("fixtures/basic-chart", &generated).unwrap();
    let lib_rs = generated.join("src/lib.rs");
    let source = std::fs::read_to_string(&lib_rs).unwrap();
    std::fs::write(
        &lib_rs,
        source.replace("abi_version: ABI_VERSION,", "abi_version: ABI_VERSION + 1,"),
    )
    .unwrap();

    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(&generated)
        .status()
        .unwrap();
    assert!(status.success());

    let error = match LoadedChartModule::load(dynamic_library_path(&generated, "basic_chart")) {
        Ok(_) => panic!("module with mismatched abi should not load"),
        Err(error) => error,
    };

    match error {
        LoadError::AbiVersionMismatch { expected, actual } => {
            assert_eq!(expected, chartwright_abi::ABI_VERSION);
            assert_eq!(actual, chartwright_abi::ABI_VERSION + 1);
        }
        other => panic!("expected abi version mismatch, got {other:?}"),
    }
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

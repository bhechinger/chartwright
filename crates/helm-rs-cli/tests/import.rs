use helm_rs_cli::import_chart;

#[test]
fn imports_basic_chart_to_generated_crate() {
    let temp = tempfile::tempdir().unwrap();
    let out_dir = temp.path().join("generated-basic-chart");

    import_chart("../../fixtures/basic-chart", &out_dir).unwrap();

    let cargo_toml = std::fs::read_to_string(out_dir.join("Cargo.toml")).unwrap();
    let lib_rs = std::fs::read_to_string(out_dir.join("src/lib.rs")).unwrap();

    assert!(cargo_toml.contains("crate-type = [\"cdylib\", \"rlib\"]"));
    assert!(lib_rs.contains("helm_rs_render_json"));
    assert!(lib_rs.contains("Chart.yaml"));
    assert!(lib_rs.contains("templates/configmap.yaml"));
    assert!(lib_rs.contains("templates/_helpers.tpl"));
}

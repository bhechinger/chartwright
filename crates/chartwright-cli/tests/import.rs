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

use chartwright_runtime::{
    render_chart, CapabilitiesInput, Chart, ChartFile, ReleaseInput, RenderInput,
};
use serde_json::json;

fn fixture(path: &str) -> String {
    std::fs::read_to_string(format!("../../fixtures/basic-chart/{path}")).unwrap()
}

#[test]
fn renders_basic_chart() {
    let chart = Chart {
        files: vec![
            ChartFile::new("Chart.yaml", fixture("Chart.yaml")),
            ChartFile::new("values.yaml", fixture("values.yaml")),
            ChartFile::new("templates/_helpers.tpl", fixture("templates/_helpers.tpl")),
            ChartFile::new(
                "templates/configmap.yaml",
                fixture("templates/configmap.yaml"),
            ),
        ],
    };
    let input = RenderInput {
        release: ReleaseInput {
            name: "demo".to_owned(),
            namespace: "testing".to_owned(),
        },
        values: json!({}),
        capabilities: CapabilitiesInput {
            kube_version: "1.30.0".to_owned(),
            api_versions: vec!["v1".to_owned()],
        },
    };

    let rendered = render_chart(&chart, &input).unwrap();

    assert_eq!(rendered, fixture("golden.yaml"));
}

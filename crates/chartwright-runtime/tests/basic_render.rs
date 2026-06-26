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

fn minimal_input() -> RenderInput {
    RenderInput {
        release: ReleaseInput {
            name: "demo".to_owned(),
            namespace: "testing".to_owned(),
        },
        values: json!({}),
        capabilities: CapabilitiesInput {
            kube_version: "1.30.0".to_owned(),
            api_versions: vec!["v1".to_owned()],
        },
    }
}

#[test]
fn renders_template_comments_as_empty_actions() {
    let chart = Chart {
        files: vec![
            ChartFile::new("Chart.yaml", "apiVersion: v2\nname: comments\nversion: 0.1.0\n"),
            ChartFile::new(
                "templates/configmap.yaml",
                "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: demo\n{{/* comment */}}\ndata:\n  key: value\n",
            ),
        ],
    };

    let rendered = render_chart(&chart, &minimal_input()).unwrap();

    assert!(rendered.contains("name: demo"));
    assert!(rendered.contains("key: value"));
    assert!(!rendered.contains("comment"));
}

#[test]
fn accepts_helper_end_marker_variants() {
    for end_marker in ["{{end}}", "{{- end}}", "{{end -}}"] {
        let chart = Chart {
            files: vec![
                ChartFile::new(
                    "Chart.yaml",
                    "apiVersion: v2\nname: helpers\nversion: 0.1.0\n",
                ),
                ChartFile::new(
                    "templates/_helpers.tpl",
                    format!("{{{{ define \"helpers.name\" }}}}demo{end_marker}"),
                ),
                ChartFile::new(
                    "templates/configmap.yaml",
                    "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {{ include \"helpers.name\" . }}\n",
                ),
            ],
        };

        let rendered = render_chart(&chart, &minimal_input()).unwrap();

        assert!(rendered.contains("name: demo"), "{end_marker}");
    }
}

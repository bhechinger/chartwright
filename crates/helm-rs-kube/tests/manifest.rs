use helm_rs_kube::{
    apply_order, inject_tracking_metadata, parse_rendered_manifest, prune_order, release_id,
    ApplyOptions, ChartIdentity, OwnerIdentity, ReleaseIdentity, ResourceAction,
};

fn options() -> ApplyOptions {
    ApplyOptions {
        release: ReleaseIdentity {
            name: "demo".to_owned(),
            namespace: "apps".to_owned(),
        },
        owner: Some(OwnerIdentity {
            api_version: "example.com/v1".to_owned(),
            kind: "ChartDeployment".to_owned(),
            name: "demo".to_owned(),
            uid: "owner-uid".to_owned(),
            namespace: Some("apps".to_owned()),
        }),
        chart: Some(ChartIdentity {
            name: "basic-chart".to_owned(),
            version: "0.1.0".to_owned(),
        }),
        field_manager: "helm-rs".to_owned(),
        force_conflicts: false,
        prune: true,
        inventory_namespace: None,
        dry_run: false,
    }
}

#[test]
fn parses_rendered_yaml_into_canonical_resources() {
    let resources = parse_rendered_manifest(
        r#"
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
data:
  key: value
---
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: widgets.example.com
"#,
        "apps",
    )
    .unwrap();

    assert_eq!(resources.len(), 2);
    assert_eq!(resources[0].id.kind, "ConfigMap");
    assert_eq!(resources[0].id.namespace.as_deref(), Some("apps"));
    assert_eq!(resources[0].id.canonical(), "/v1/ConfigMap apps/app-config");
    assert_eq!(resources[1].id.group, "apiextensions.k8s.io");
    assert_eq!(resources[1].id.namespace, None);
}

#[test]
fn injects_tracking_metadata_and_valid_owner_reference() {
    let mut resources = parse_rendered_manifest(
        r#"
apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
  namespace: apps
"#,
        "apps",
    )
    .unwrap();
    let report = inject_tracking_metadata(&mut resources[0], &options()).unwrap();

    assert_eq!(report.action, ResourceAction::Skipped);
    assert_eq!(
        resources[0].value["metadata"]["labels"]["app.kubernetes.io/managed-by"],
        "helm-rs"
    );
    assert_eq!(
        resources[0].value["metadata"]["annotations"]["helm-rs.io/chart-name"],
        "basic-chart"
    );
    assert_eq!(
        resources[0].value["metadata"]["ownerReferences"][0]["uid"],
        "owner-uid"
    );
}

#[test]
fn release_id_is_stable_and_short() {
    let left = release_id("demo", "apps");
    let right = release_id("demo", "apps");

    assert_eq!(left, right);
    assert_eq!(left.len(), 12);
}

#[test]
fn orders_apply_and_prune_for_dependencies() {
    let resources = parse_rendered_manifest(
        r#"
---
apiVersion: example.com/v1
kind: Widget
metadata:
  name: one
  namespace: apps
---
apiVersion: v1
kind: Namespace
metadata:
  name: apps
---
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: widgets.example.com
"#,
        "apps",
    )
    .unwrap();

    let apply = apply_order(resources.clone());
    assert_eq!(apply[0].id.kind, "Namespace");
    assert_eq!(apply[1].id.kind, "CustomResourceDefinition");
    assert_eq!(apply[2].id.kind, "Widget");

    let prune = prune_order(resources);
    assert_eq!(prune[0].id.kind, "Widget");
    assert_eq!(prune[1].id.kind, "CustomResourceDefinition");
    assert_eq!(prune[2].id.kind, "Namespace");
}

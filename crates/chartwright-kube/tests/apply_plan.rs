use chartwright_events::{Event, InMemoryEventSink};
use chartwright_kube::{
    build_apply_plan, ApplyOptions, ChartIdentity, ReleaseIdentity, ResourceAction,
};

fn options() -> ApplyOptions {
    ApplyOptions {
        release: ReleaseIdentity {
            name: "demo".to_owned(),
            namespace: "apps".to_owned(),
        },
        owner: None,
        chart: Some(ChartIdentity {
            name: "basic-chart".to_owned(),
            version: "0.1.0".to_owned(),
        }),
        field_manager: "chartwright".to_owned(),
        force_conflicts: false,
        prune: true,
        inventory_namespace: None,
        dry_run: true,
    }
}

#[test]
fn build_apply_plan_reports_dry_run_apply_and_inventory() {
    let events = InMemoryEventSink::default();
    let report = build_apply_plan(
        r#"
apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
"#,
        None,
        options(),
        events.clone(),
    )
    .unwrap();

    assert_eq!(report.applied.len(), 1);
    assert_eq!(report.applied[0].action, ResourceAction::Applied);
    assert_eq!(
        report.inventory.unwrap().action,
        ResourceAction::InventoryUpdated
    );
    assert!(events.events().iter().any(|event| matches!(
        event,
        Event::StepDetail { detail, .. } if detail == "parsed 1 desired resources"
    )));
}

#[test]
fn build_apply_plan_reports_pruned_inventory_resources() {
    let previous = chartwright_kube::Inventory {
        release_id: options().release_id(),
        resources: vec![chartwright_kube::ResourceId {
            group: String::new(),
            version: "v1".to_owned(),
            kind: "Secret".to_owned(),
            namespace: Some("apps".to_owned()),
            name: "stale".to_owned(),
        }],
    };

    let report =
        build_apply_plan("", Some(previous), options(), InMemoryEventSink::default()).unwrap();

    assert_eq!(report.applied.len(), 0);
    assert_eq!(report.pruned.len(), 1);
    assert_eq!(report.pruned[0].id.kind, "Secret");
    assert_eq!(report.pruned[0].action, ResourceAction::Pruned);
}

use helm_rs_kube::{
    inventory_from_json, inventory_from_resources, inventory_name, inventory_to_json,
    resources_to_prune, ApplyOptions, Inventory, ResourceId,
};

fn options() -> ApplyOptions {
    ApplyOptions {
        release: helm_rs_kube::ReleaseIdentity {
            name: "demo".to_owned(),
            namespace: "apps".to_owned(),
        },
        owner: None,
        chart: None,
        field_manager: "helm-rs".to_owned(),
        force_conflicts: false,
        prune: true,
        inventory_namespace: None,
        dry_run: false,
    }
}

fn id(kind: &str, name: &str) -> ResourceId {
    ResourceId {
        group: String::new(),
        version: "v1".to_owned(),
        kind: kind.to_owned(),
        namespace: Some("apps".to_owned()),
        name: name.to_owned(),
    }
}

#[test]
fn inventory_round_trips_and_finds_stale_resources() {
    let desired = vec![helm_rs_kube::ManifestResource {
        id: id("ConfigMap", "current"),
        value: serde_json::json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {"name": "current", "namespace": "apps"}
        }),
    }];
    let inventory = Inventory {
        release_id: options().release_id(),
        resources: vec![id("ConfigMap", "current"), id("Secret", "stale")],
    };

    let encoded = inventory_to_json(&inventory).unwrap();
    let decoded = inventory_from_json(&encoded).unwrap();
    let stale = resources_to_prune(&decoded, &desired);

    assert_eq!(
        inventory_name(&decoded.release_id),
        "helm-rs-inventory-98ab58245c53"
    );
    assert_eq!(decoded, inventory);
    assert_eq!(stale, vec![id("Secret", "stale")]);
    assert_eq!(
        inventory_from_resources(&options(), &desired).resources,
        vec![id("ConfigMap", "current")]
    );
}

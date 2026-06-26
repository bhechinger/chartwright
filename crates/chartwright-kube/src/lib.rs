use chartwright_events::{Event, EventLevel, EventSink};
use kube::{
    api::{Api, DeleteParams, DynamicObject, Patch, PatchParams},
    core::GroupVersionKind,
    discovery::{self, Scope},
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseIdentity {
    pub name: String,
    pub namespace: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerIdentity {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub uid: String,
    pub namespace: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChartIdentity {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyOptions {
    pub release: ReleaseIdentity,
    pub owner: Option<OwnerIdentity>,
    pub chart: Option<ChartIdentity>,
    pub field_manager: String,
    pub force_conflicts: bool,
    pub prune: bool,
    pub inventory_namespace: Option<String>,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteOptions {
    pub release: ReleaseIdentity,
    pub inventory_namespace: Option<String>,
    pub dry_run: bool,
}

impl DeleteOptions {
    pub fn release_id(&self) -> String {
        release_id(&self.release.name, &self.release.namespace)
    }

    pub fn inventory_namespace(&self) -> &str {
        self.inventory_namespace
            .as_deref()
            .unwrap_or(&self.release.namespace)
    }
}

impl ApplyOptions {
    pub fn release_id(&self) -> String {
        release_id(&self.release.name, &self.release.namespace)
    }

    pub fn inventory_namespace(&self) -> &str {
        self.inventory_namespace
            .as_deref()
            .unwrap_or(&self.release.namespace)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ResourceId {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
}

impl ResourceId {
    pub fn canonical(&self) -> String {
        format!(
            "{}/{}/{} {}/{}",
            self.group,
            self.version,
            self.kind,
            self.namespace.as_deref().unwrap_or("-"),
            self.name
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ManifestResource {
    pub id: ResourceId,
    pub value: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceAction {
    Applied,
    Pruned,
    Skipped,
    InventoryUpdated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceStatus {
    pub id: ResourceId,
    pub action: ResourceAction,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApplyReport {
    pub applied: Vec<ResourceStatus>,
    pub pruned: Vec<ResourceStatus>,
    pub inventory: Option<ResourceStatus>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeleteReport {
    pub deleted: Vec<ResourceStatus>,
    pub inventory: Option<ResourceStatus>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Error)]
pub enum KubeApplyError {
    #[error("manifest document {index} is invalid yaml: {source}")]
    InvalidYaml {
        index: usize,
        source: serde_yaml::Error,
    },
    #[error("manifest document {index} must be a kubernetes object")]
    InvalidObject { index: usize },
    #[error("manifest document {index} is a List object; render individual resources instead")]
    ListObject { index: usize },
    #[error("manifest document {index} is missing {field}")]
    MissingField { index: usize, field: &'static str },
    #[error("invalid apiVersion {api_version} in manifest document {index}")]
    InvalidApiVersion { index: usize, api_version: String },
    #[error("owner reference is not valid for {resource}: {reason}")]
    InvalidOwnerReference { resource: String, reason: String },
    #[error("inventory payload is invalid: {0}")]
    InvalidInventory(serde_json::Error),
    #[error("kubernetes api error for {resource}: {source}")]
    Kube {
        resource: String,
        source: kube::Error,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Inventory {
    pub release_id: String,
    pub resources: Vec<ResourceId>,
}

pub fn release_id(name: &str, namespace: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(namespace.as_bytes());
    hash.update(b"/");
    hash.update(name.as_bytes());
    hex::encode(hash.finalize())[..12].to_owned()
}

pub fn parse_rendered_manifest(
    rendered_yaml: &str,
    default_namespace: &str,
) -> Result<Vec<ManifestResource>, KubeApplyError> {
    let mut resources = Vec::new();
    for (index, document) in serde_yaml::Deserializer::from_str(rendered_yaml).enumerate() {
        let yaml_value = Value::deserialize(document)
            .map_err(|source| KubeApplyError::InvalidYaml { index, source })?;
        if yaml_value.is_null() {
            continue;
        }
        let mut object = match yaml_value {
            Value::Object(object) if object.is_empty() => continue,
            Value::Object(object) => object,
            _ => return Err(KubeApplyError::InvalidObject { index }),
        };

        let api_version = required_string(&object, "apiVersion", index)?.to_owned();
        let kind = required_string(&object, "kind", index)?.to_owned();
        if kind == "List" {
            return Err(KubeApplyError::ListObject { index });
        }
        let (group, version) = split_api_version(index, &api_version)?;
        let metadata = object
            .entry("metadata")
            .or_insert_with(|| Value::Object(Map::new()));
        let metadata_object = metadata
            .as_object_mut()
            .ok_or(KubeApplyError::MissingField {
                index,
                field: "metadata",
            })?;
        let name = metadata_object
            .get("name")
            .and_then(Value::as_str)
            .ok_or(KubeApplyError::MissingField {
                index,
                field: "metadata.name",
            })?
            .to_owned();
        let namespace = resource_namespace(&kind, metadata_object, default_namespace);
        if let Some(namespace) = &namespace {
            metadata_object
                .entry("namespace")
                .or_insert_with(|| Value::String(namespace.clone()));
        }

        resources.push(ManifestResource {
            id: ResourceId {
                group,
                version,
                kind,
                namespace,
                name,
            },
            value: Value::Object(object),
        });
    }
    Ok(resources)
}

pub fn inject_tracking_metadata(
    resource: &mut ManifestResource,
    options: &ApplyOptions,
) -> Result<ResourceStatus, KubeApplyError> {
    let release_id = options.release_id();
    let object = resource
        .value
        .as_object_mut()
        .expect("ManifestResource value is always an object");
    let metadata = object
        .entry("metadata")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .expect("metadata was validated as an object");

    insert_map_value(
        metadata,
        "labels",
        "app.kubernetes.io/managed-by",
        "chartwright",
    );
    insert_map_value(metadata, "labels", "chartwright.io/release-id", &release_id);
    insert_map_value(
        metadata,
        "annotations",
        "chartwright.io/release-name",
        &options.release.name,
    );
    insert_map_value(
        metadata,
        "annotations",
        "chartwright.io/release-namespace",
        &options.release.namespace,
    );
    insert_map_value(
        metadata,
        "annotations",
        "chartwright.io/resource-id",
        &resource.id.canonical(),
    );
    if let Some(chart) = &options.chart {
        insert_map_value(
            metadata,
            "annotations",
            "chartwright.io/chart-name",
            &chart.name,
        );
        insert_map_value(
            metadata,
            "annotations",
            "chartwright.io/chart-version",
            &chart.version,
        );
    }
    if let Some(owner) = &options.owner {
        insert_map_value(
            metadata,
            "annotations",
            "chartwright.io/owner-uid",
            &owner.uid,
        );
        if owner_reference_allowed(owner, &resource.id) {
            metadata.insert(
                "ownerReferences".to_owned(),
                Value::Array(vec![serde_json::json!({
                    "apiVersion": owner.api_version,
                    "kind": owner.kind,
                    "name": owner.name,
                    "uid": owner.uid,
                    "controller": true,
                    "blockOwnerDeletion": true,
                })]),
            );
        }
    }

    Ok(ResourceStatus {
        id: resource.id.clone(),
        action: ResourceAction::Skipped,
        message: Some("tracking metadata injected".to_owned()),
    })
}

pub fn apply_order(mut resources: Vec<ManifestResource>) -> Vec<ManifestResource> {
    resources.sort_by(|left, right| {
        apply_rank(&left.id)
            .cmp(&apply_rank(&right.id))
            .then_with(|| left.id.canonical().cmp(&right.id.canonical()))
    });
    resources
}

pub fn prune_order(mut resources: Vec<ManifestResource>) -> Vec<ManifestResource> {
    resources.sort_by(|left, right| {
        prune_rank(&left.id)
            .cmp(&prune_rank(&right.id))
            .then_with(|| left.id.canonical().cmp(&right.id.canonical()))
    });
    resources
}

pub fn build_apply_plan<S: EventSink>(
    rendered_yaml: &str,
    previous_inventory: Option<Inventory>,
    options: ApplyOptions,
    events: S,
) -> Result<ApplyReport, KubeApplyError> {
    events.emit(Event::StepStarted {
        id: "apply-rendered-chart".to_owned(),
        label: "apply rendered chart".to_owned(),
        detail: Some(format!(
            "{} in namespace {}",
            options.release.name, options.release.namespace
        )),
    });
    let mut desired = parse_rendered_manifest(rendered_yaml, &options.release.namespace)?;
    events.emit(Event::StepDetail {
        id: "apply-rendered-chart".to_owned(),
        detail: format!("parsed {} desired resources", desired.len()),
    });
    for resource in &mut desired {
        inject_tracking_metadata(resource, &options)?;
    }
    let desired = apply_order(desired);
    let stale = previous_inventory
        .as_ref()
        .filter(|_| options.prune)
        .map(|inventory| resources_to_prune(inventory, &desired))
        .unwrap_or_default();

    let applied = desired
        .iter()
        .map(|resource| ResourceStatus {
            id: resource.id.clone(),
            action: ResourceAction::Applied,
            message: Some(if options.dry_run {
                "planned server-side apply".to_owned()
            } else {
                "server-side apply pending".to_owned()
            }),
        })
        .collect();
    let pruned = stale
        .into_iter()
        .map(|id| ResourceStatus {
            id,
            action: ResourceAction::Pruned,
            message: Some(if options.dry_run {
                "planned prune".to_owned()
            } else {
                "prune pending".to_owned()
            }),
        })
        .collect();
    let inventory = Some(ResourceStatus {
        id: inventory_resource_id(&options),
        action: ResourceAction::InventoryUpdated,
        message: Some("planned inventory update".to_owned()),
    });

    events.emit(Event::Log {
        level: EventLevel::Info,
        message: "built apply plan".to_owned(),
    });
    events.emit(Event::StepFinished {
        id: "apply-rendered-chart".to_owned(),
        message: "built apply plan".to_owned(),
        elapsed: std::time::Duration::ZERO,
    });

    Ok(ApplyReport {
        applied,
        pruned,
        inventory,
        warnings: Vec::new(),
    })
}

pub async fn apply_rendered_chart<S: EventSink>(
    client: Client,
    rendered_yaml: &str,
    options: ApplyOptions,
    events: S,
) -> Result<ApplyReport, KubeApplyError> {
    if options.dry_run {
        let previous_inventory = if options.prune {
            read_inventory(client, &options).await?
        } else {
            None
        };
        return build_apply_plan(rendered_yaml, previous_inventory, options, events);
    }

    events.emit(Event::StepStarted {
        id: "apply-rendered-chart".to_owned(),
        label: "apply rendered chart".to_owned(),
        detail: Some(format!(
            "{} in namespace {}",
            options.release.name, options.release.namespace
        )),
    });

    let previous_inventory = read_inventory(client.clone(), &options).await?;
    let mut desired = parse_rendered_manifest(rendered_yaml, &options.release.namespace)?;
    events.emit(Event::StepDetail {
        id: "apply-rendered-chart".to_owned(),
        detail: format!("parsed {} desired resources", desired.len()),
    });
    normalize_resource_scopes(client.clone(), &mut desired).await?;
    for resource in &mut desired {
        inject_tracking_metadata(resource, &options)?;
    }
    let desired = apply_order(desired);
    let mut applied = Vec::new();
    for resource in &desired {
        apply_resource(client.clone(), resource, &options).await?;
        events.emit(Event::StepDetail {
            id: "apply-rendered-chart".to_owned(),
            detail: format!("applied {}", resource.id.canonical()),
        });
        applied.push(ResourceStatus {
            id: resource.id.clone(),
            action: ResourceAction::Applied,
            message: Some("server-side apply complete".to_owned()),
        });
    }

    let mut pruned = Vec::new();
    if options.prune {
        if let Some(previous) = &previous_inventory {
            for id in prune_resource_ids(resources_to_prune(previous, &desired)) {
                delete_resource(client.clone(), &id).await?;
                events.emit(Event::StepDetail {
                    id: "apply-rendered-chart".to_owned(),
                    detail: format!("pruned {}", id.canonical()),
                });
                pruned.push(ResourceStatus {
                    id,
                    action: ResourceAction::Pruned,
                    message: Some("delete requested".to_owned()),
                });
            }
        }
    }

    let inventory = inventory_from_resources(&options, &desired);
    write_inventory(client, &options, &inventory).await?;
    let inventory_status = ResourceStatus {
        id: inventory_resource_id(&options),
        action: ResourceAction::InventoryUpdated,
        message: Some("inventory updated".to_owned()),
    };
    events.emit(Event::StepFinished {
        id: "apply-rendered-chart".to_owned(),
        message: "applied rendered chart".to_owned(),
        elapsed: std::time::Duration::ZERO,
    });

    Ok(ApplyReport {
        applied,
        pruned,
        inventory: Some(inventory_status),
        warnings: Vec::new(),
    })
}

pub async fn delete_release<S: EventSink>(
    client: Client,
    options: DeleteOptions,
    events: S,
) -> Result<DeleteReport, KubeApplyError> {
    events.emit(Event::StepStarted {
        id: "delete-release".to_owned(),
        label: "delete release".to_owned(),
        detail: Some(format!(
            "{} in namespace {}",
            options.release.name, options.release.namespace
        )),
    });
    let inventory = read_inventory_for_delete(client.clone(), &options).await?;
    let mut deleted = Vec::new();
    if let Some(inventory) = inventory {
        for id in prune_resource_ids(inventory.resources) {
            if !options.dry_run {
                delete_resource(client.clone(), &id).await?;
            }
            events.emit(Event::StepDetail {
                id: "delete-release".to_owned(),
                detail: format!("deleted {}", id.canonical()),
            });
            deleted.push(ResourceStatus {
                id,
                action: ResourceAction::Pruned,
                message: Some(if options.dry_run {
                    "planned delete".to_owned()
                } else {
                    "delete requested".to_owned()
                }),
            });
        }
    }
    let inventory_id = delete_inventory_resource_id(&options);
    if !options.dry_run {
        delete_resource(client, &inventory_id).await?;
    }
    events.emit(Event::StepFinished {
        id: "delete-release".to_owned(),
        message: "deleted release resources".to_owned(),
        elapsed: std::time::Duration::ZERO,
    });
    Ok(DeleteReport {
        deleted,
        inventory: Some(ResourceStatus {
            id: inventory_id,
            action: ResourceAction::Pruned,
            message: Some("inventory deleted".to_owned()),
        }),
        warnings: Vec::new(),
    })
}

pub fn inventory_name(release_id: &str) -> String {
    format!("chartwright-inventory-{release_id}")
}

pub fn inventory_resource_id(options: &ApplyOptions) -> ResourceId {
    ResourceId {
        group: String::new(),
        version: "v1".to_owned(),
        kind: "ConfigMap".to_owned(),
        namespace: Some(options.inventory_namespace().to_owned()),
        name: inventory_name(&options.release_id()),
    }
}

fn delete_inventory_resource_id(options: &DeleteOptions) -> ResourceId {
    ResourceId {
        group: String::new(),
        version: "v1".to_owned(),
        kind: "ConfigMap".to_owned(),
        namespace: Some(options.inventory_namespace().to_owned()),
        name: inventory_name(&options.release_id()),
    }
}

fn prune_resource_ids(ids: Vec<ResourceId>) -> Vec<ResourceId> {
    let resources: Vec<ManifestResource> = ids
        .into_iter()
        .map(|id| ManifestResource {
            id,
            value: Value::Object(Map::new()),
        })
        .collect();
    prune_order(resources)
        .into_iter()
        .map(|resource| resource.id)
        .collect()
}

async fn apply_resource(
    client: Client,
    resource: &ManifestResource,
    options: &ApplyOptions,
) -> Result<(), KubeApplyError> {
    let (api, name, scope) = dynamic_api(client, &resource.id).await?;
    let mut params = PatchParams::apply(&options.field_manager);
    if options.force_conflicts {
        params = params.force();
    }
    if options.dry_run {
        params = params.dry_run();
    }
    let mut value = resource.value.clone();
    if matches!(scope, Scope::Cluster) {
        remove_metadata_namespace(&mut value);
    }
    let object: DynamicObject =
        serde_json::from_value(value).map_err(KubeApplyError::InvalidInventory)?;
    api.patch(&name, &params, &Patch::Apply(&object))
        .await
        .map_err(|source| KubeApplyError::Kube {
            resource: resource.id.canonical(),
            source,
        })?;
    Ok(())
}

async fn delete_resource(client: Client, id: &ResourceId) -> Result<(), KubeApplyError> {
    let (api, name, _) = dynamic_api(client, id).await?;
    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(error)) if error.code == 404 => Ok(()),
        Err(source) => Err(KubeApplyError::Kube {
            resource: id.canonical(),
            source,
        }),
    }
}

async fn dynamic_api(
    client: Client,
    id: &ResourceId,
) -> Result<(Api<DynamicObject>, String, Scope), KubeApplyError> {
    let gvk = GroupVersionKind::gvk(&id.group, &id.version, &id.kind);
    let (api_resource, capabilities) =
        discovery::pinned_kind(&client, &gvk)
            .await
            .map_err(|source| KubeApplyError::Kube {
                resource: id.canonical(),
                source,
            })?;
    let api = match capabilities.scope {
        Scope::Cluster => Api::all_with(client, &api_resource),
        Scope::Namespaced => Api::namespaced_with(
            client,
            id.namespace.as_deref().unwrap_or("default"),
            &api_resource,
        ),
    };
    Ok((api, id.name.clone(), capabilities.scope))
}

async fn normalize_resource_scopes(
    client: Client,
    resources: &mut [ManifestResource],
) -> Result<(), KubeApplyError> {
    for resource in resources {
        let gvk =
            GroupVersionKind::gvk(&resource.id.group, &resource.id.version, &resource.id.kind);
        let (_, capabilities) = discovery::pinned_kind(&client, &gvk)
            .await
            .map_err(|source| KubeApplyError::Kube {
                resource: resource.id.canonical(),
                source,
            })?;
        match capabilities.scope {
            Scope::Cluster => {
                resource.id.namespace = None;
                remove_metadata_namespace(&mut resource.value);
            }
            Scope::Namespaced => {
                if resource.id.namespace.is_none() {
                    resource.id.namespace = Some("default".to_owned());
                }
            }
        }
    }
    Ok(())
}

async fn read_inventory(
    client: Client,
    options: &ApplyOptions,
) -> Result<Option<Inventory>, KubeApplyError> {
    read_inventory_by_id(client, &inventory_resource_id(options)).await
}

async fn read_inventory_for_delete(
    client: Client,
    options: &DeleteOptions,
) -> Result<Option<Inventory>, KubeApplyError> {
    read_inventory_by_id(client, &delete_inventory_resource_id(options)).await
}

async fn read_inventory_by_id(
    client: Client,
    id: &ResourceId,
) -> Result<Option<Inventory>, KubeApplyError> {
    let (api, name, _) = dynamic_api(client, id).await?;
    let object = api
        .get_opt(&name)
        .await
        .map_err(|source| KubeApplyError::Kube {
            resource: id.canonical(),
            source,
        })?;
    let Some(object) = object else {
        return Ok(None);
    };
    let Some(inventory_json) = object
        .data
        .get("data")
        .and_then(Value::as_object)
        .and_then(|data| data.get("inventory.json"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    inventory_from_json(inventory_json).map(Some)
}

async fn write_inventory(
    client: Client,
    options: &ApplyOptions,
    inventory: &Inventory,
) -> Result<(), KubeApplyError> {
    let id = inventory_resource_id(options);
    let inventory_json = inventory_to_json(inventory)?;
    let value = serde_json::json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {
            "name": id.name,
            "namespace": id.namespace,
            "labels": {
                "app.kubernetes.io/managed-by": "chartwright",
                "chartwright.io/release-id": options.release_id(),
            },
            "annotations": {
                "chartwright.io/release-name": options.release.name,
                "chartwright.io/release-namespace": options.release.namespace,
            }
        },
        "data": {
            "inventory.json": inventory_json,
        }
    });
    let resource = ManifestResource { id, value };
    apply_resource(client, &resource, options).await
}

fn remove_metadata_namespace(value: &mut Value) {
    if let Some(metadata) = value
        .as_object_mut()
        .and_then(|object| object.get_mut("metadata"))
        .and_then(Value::as_object_mut)
    {
        metadata.remove("namespace");
    }
}

pub fn inventory_from_resources(
    options: &ApplyOptions,
    resources: &[ManifestResource],
) -> Inventory {
    Inventory {
        release_id: options.release_id(),
        resources: resources
            .iter()
            .map(|resource| resource.id.clone())
            .collect(),
    }
}

pub fn inventory_to_json(inventory: &Inventory) -> Result<String, KubeApplyError> {
    serde_json::to_string_pretty(inventory).map_err(KubeApplyError::InvalidInventory)
}

pub fn inventory_from_json(json: &str) -> Result<Inventory, KubeApplyError> {
    serde_json::from_str(json).map_err(KubeApplyError::InvalidInventory)
}

pub fn resources_to_prune(previous: &Inventory, desired: &[ManifestResource]) -> Vec<ResourceId> {
    let desired: std::collections::BTreeSet<ResourceId> =
        desired.iter().map(|resource| resource.id.clone()).collect();
    let mut stale: Vec<ResourceId> = previous
        .resources
        .iter()
        .filter(|resource| !desired.contains(resource))
        .cloned()
        .collect();
    stale.sort();
    stale
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
    index: usize,
) -> Result<&'a str, KubeApplyError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(KubeApplyError::MissingField { index, field })
}

fn split_api_version(index: usize, api_version: &str) -> Result<(String, String), KubeApplyError> {
    if let Some((group, version)) = api_version.split_once('/') {
        if group.is_empty() || version.is_empty() {
            return Err(KubeApplyError::InvalidApiVersion {
                index,
                api_version: api_version.to_owned(),
            });
        }
        Ok((group.to_owned(), version.to_owned()))
    } else if api_version.is_empty() {
        Err(KubeApplyError::InvalidApiVersion {
            index,
            api_version: api_version.to_owned(),
        })
    } else {
        Ok(("".to_owned(), api_version.to_owned()))
    }
}

fn resource_namespace(
    kind: &str,
    metadata: &Map<String, Value>,
    default_namespace: &str,
) -> Option<String> {
    if is_cluster_scoped_kind(kind) {
        None
    } else {
        Some(
            metadata
                .get("namespace")
                .and_then(Value::as_str)
                .unwrap_or(default_namespace)
                .to_owned(),
        )
    }
}

fn is_cluster_scoped_kind(kind: &str) -> bool {
    matches!(
        kind,
        "Namespace"
            | "CustomResourceDefinition"
            | "ClusterRole"
            | "ClusterRoleBinding"
            | "StorageClass"
            | "PersistentVolume"
            | "Node"
            | "MutatingWebhookConfiguration"
            | "ValidatingWebhookConfiguration"
            | "GatewayClass"
    )
}

fn insert_map_value(
    metadata: &mut Map<String, Value>,
    parent: &'static str,
    key: &'static str,
    value: &str,
) {
    let parent = metadata
        .entry(parent)
        .or_insert_with(|| Value::Object(Map::new()));
    let parent = parent
        .as_object_mut()
        .expect("metadata child maps are controlled by chartwright");
    parent.insert(key.to_owned(), Value::String(value.to_owned()));
}

fn owner_reference_allowed(owner: &OwnerIdentity, resource: &ResourceId) -> bool {
    match (&owner.namespace, &resource.namespace) {
        (None, _) => true,
        (Some(owner_namespace), Some(resource_namespace)) => owner_namespace == resource_namespace,
        (Some(_), None) => false,
    }
}

fn apply_rank(id: &ResourceId) -> u8 {
    match id.kind.as_str() {
        "Namespace" => 0,
        "CustomResourceDefinition" => 1,
        _ => 2,
    }
}

fn prune_rank(id: &ResourceId) -> u8 {
    match id.kind.as_str() {
        "Namespace" => 2,
        "CustomResourceDefinition" => 1,
        _ => 0,
    }
}

use helm_rs_abi::{
    buffer_from_bytes, error_buffer, free_buffer, AbiBuffer, AbiResult, ModuleInfo, RenderRequest,
};

#[test]
fn owned_buffer_round_trips_bytes() {
    let buffer = buffer_from_bytes(b"hello");

    assert!(!buffer.ptr.is_null());
    assert_eq!(buffer.len, 5);
    assert_eq!(unsafe { buffer.as_slice() }, b"hello");

    unsafe { free_buffer(buffer) };
}

#[test]
fn null_buffer_is_empty_slice() {
    let buffer = AbiBuffer {
        ptr: std::ptr::null_mut(),
        len: 0,
        capacity: 0,
    };

    assert_eq!(unsafe { buffer.as_slice() }, b"");
}

#[test]
fn serializes_shared_json_types() {
    let request = RenderRequest {
        release_name: "demo".to_owned(),
        namespace: "testing".to_owned(),
        values: serde_json::json!({"name": "override"}),
        kube_version: "1.30.0".to_owned(),
        api_versions: vec!["v1".to_owned()],
    };
    let info = ModuleInfo {
        abi_version: 1,
        chart_name: "basic-chart".to_owned(),
        chart_version: "0.1.0".to_owned(),
        runtime_version: "0.1.0".to_owned(),
    };

    assert!(serde_json::to_string(&request)
        .unwrap()
        .contains("release_name"));
    assert!(serde_json::to_string(&info)
        .unwrap()
        .contains("basic-chart"));
    assert_eq!(AbiResult::ok(buffer_from_bytes(b"ok")).code, 0);
}

#[test]
fn serializes_structured_error_buffer() {
    let buffer = error_buffer("render_error", "unsupported function");
    let bytes = unsafe { buffer.as_slice() };
    let error: serde_json::Value = serde_json::from_slice(bytes).unwrap();

    assert_eq!(error["code"], "render_error");
    assert_eq!(error["message"], "unsupported function");

    unsafe { free_buffer(buffer) };
}

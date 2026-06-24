use serde::{Deserialize, Serialize};

pub const ABI_VERSION: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AbiBuffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub capacity: usize,
}

impl AbiBuffer {
    pub const fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
            capacity: 0,
        }
    }

    /// # Safety
    ///
    /// The pointer must either be null with length 0 or refer to a live buffer
    /// allocated by this ABI module.
    pub unsafe fn as_slice(&self) -> &[u8] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(self.ptr.cast_const(), self.len)
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AbiResult {
    pub code: i32,
    pub buffer: AbiBuffer,
}

impl AbiResult {
    pub const fn ok(buffer: AbiBuffer) -> Self {
        Self { code: 0, buffer }
    }

    pub const fn err(buffer: AbiBuffer) -> Self {
        Self { code: 1, buffer }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RenderRequest {
    pub release_name: String,
    pub namespace: String,
    #[serde(default)]
    pub values: serde_json::Value,
    pub kube_version: String,
    #[serde(default)]
    pub api_versions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModuleInfo {
    pub abi_version: u32,
    pub chart_name: String,
    pub chart_version: String,
    pub runtime_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AbiError {
    pub code: String,
    pub message: String,
}

pub fn error_buffer(code: impl Into<String>, message: impl Into<String>) -> AbiBuffer {
    let error = AbiError {
        code: code.into(),
        message: message.into(),
    };
    match serde_json::to_vec(&error) {
        Ok(bytes) => buffer_from_bytes(&bytes),
        Err(error) => buffer_from_bytes(error.to_string().as_bytes()),
    }
}

pub fn buffer_from_bytes(bytes: &[u8]) -> AbiBuffer {
    if bytes.is_empty() {
        return AbiBuffer::empty();
    }
    let mut owned = bytes.to_vec();
    let buffer = AbiBuffer {
        ptr: owned.as_mut_ptr(),
        len: owned.len(),
        capacity: owned.capacity(),
    };
    std::mem::forget(owned);
    buffer
}

/// # Safety
///
/// `buffer` must have been created by `buffer_from_bytes` in the same dynamic
/// library instance and must not already have been freed.
pub unsafe fn free_buffer(buffer: AbiBuffer) {
    if buffer.ptr.is_null() || buffer.capacity == 0 {
        return;
    }
    drop(Vec::from_raw_parts(buffer.ptr, buffer.len, buffer.capacity));
}

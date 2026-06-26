use serde::{Deserialize, Serialize};
#[cfg(feature = "loader")]
use thiserror::Error;

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

#[cfg(feature = "loader")]
type RenderJson = unsafe extern "C" fn(*const u8, usize, *mut AbiBuffer) -> i32;
#[cfg(feature = "loader")]
type ModuleInfoFn = unsafe extern "C" fn(*mut AbiBuffer) -> i32;
#[cfg(feature = "loader")]
type FreeBuffer = unsafe extern "C" fn(AbiBuffer);

#[cfg(feature = "loader")]
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("failed to load module: {0}")]
    Library(#[from] libloading::Error),
    #[error("failed to encode render request as json: {0}")]
    Encode(serde_json::Error),
    #[error("module returned non-utf8 output: {0}")]
    Utf8(std::string::FromUtf8Error),
    #[error("module returned invalid json: {0}")]
    Decode(serde_json::Error),
    #[error("module returned abi misuse code {0}")]
    Abi(i32),
    #[error("module error {code}: {message}")]
    Module { code: String, message: String },
}

#[cfg(feature = "loader")]
pub struct LoadedChartModule {
    library: libloading::Library,
}

#[cfg(feature = "loader")]
impl LoadedChartModule {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, LoadError> {
        let library = unsafe { libloading::Library::new(path.as_ref())? };
        unsafe {
            let _: libloading::Symbol<RenderJson> = library.get(b"helm_rs_render_json")?;
            let _: libloading::Symbol<ModuleInfoFn> = library.get(b"helm_rs_module_info")?;
            let _: libloading::Symbol<FreeBuffer> = library.get(b"helm_rs_free")?;
        }
        Ok(Self { library })
    }

    pub fn info(&self) -> Result<ModuleInfo, LoadError> {
        let mut output = AbiBuffer::empty();
        let code = unsafe {
            let module_info: libloading::Symbol<ModuleInfoFn> =
                self.library.get(b"helm_rs_module_info")?;
            module_info(&mut output)
        };
        let bytes = self.take_output(output)?;
        match code {
            0 => serde_json::from_slice(&bytes).map_err(LoadError::Decode),
            1 => Err(decode_module_error(&bytes)),
            code => Err(LoadError::Abi(code)),
        }
    }

    pub fn render(&self, request: RenderRequest) -> Result<String, LoadError> {
        let request = serde_json::to_vec(&request).map_err(LoadError::Encode)?;
        let mut output = AbiBuffer::empty();
        let code = unsafe {
            let render: libloading::Symbol<RenderJson> =
                self.library.get(b"helm_rs_render_json")?;
            render(request.as_ptr(), request.len(), &mut output)
        };
        let bytes = self.take_output(output)?;
        match code {
            0 => String::from_utf8(bytes).map_err(LoadError::Utf8),
            1 => Err(decode_module_error(&bytes)),
            code => Err(LoadError::Abi(code)),
        }
    }

    fn take_output(&self, output: AbiBuffer) -> Result<Vec<u8>, LoadError> {
        unsafe {
            let bytes = output.as_slice().to_vec();
            let free: libloading::Symbol<FreeBuffer> = self.library.get(b"helm_rs_free")?;
            free(output);
            Ok(bytes)
        }
    }
}

#[cfg(feature = "loader")]
fn decode_module_error(bytes: &[u8]) -> LoadError {
    match serde_json::from_slice::<AbiError>(bytes) {
        Ok(error) => LoadError::Module {
            code: error.code,
            message: error.message,
        },
        Err(error) => LoadError::Decode(error),
    }
}

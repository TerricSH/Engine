//! ILRuntime-based script host.
//!
//! [`ILRuntimeHost`] implements [`ScriptHost`] by delegating all script
//! operations to an ILRuntime-powered .NET runtime accessed through FFI.
//!
//! # Architecture
//!
//! ```text
//! Rust (ILRuntimeHost)  ─── FFI ───→  Managed (.NET CoreCLR / NativeAOT)
//!                                          └── ILRuntime AppDomain
//!                                               ├── LoadAssembly()
//!                                               ├── Instantiate()
//!                                               └── Invoke()
//! ```
//!
//! The managed side handles all IL interpretation. The Rust side provides
//! the engine API (component read/write via engine-ffi) that scripts call.
//!
//! # States
//!
//! * **Uninitialized** — created but no runtime attached.
//! * **Ready** — runtime initialized, accepting script operations.
//! * **Error** — runtime failed to initialize or crashed.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::host::{ScriptError, ScriptHandle, ScriptHost, ScriptInstance};
use crate::value::ScriptValue;

// ---------------------------------------------------------------------------
// FFI function signatures (provided by the managed side)
// ---------------------------------------------------------------------------

type FfiLoadAssembly =
    extern "C" fn(id: *const u8, id_len: u32, data: *const u8, data_len: u32) -> i32;
type FfiInstantiate = extern "C" fn(
    assembly_id: *const u8,
    id_len: u32,
    class_name: *const u8,
    name_len: u32,
    instance_id: *const u8,
    inst_len: u32,
) -> i32;
type FfiCallMethod = extern "C" fn(
    instance_id: *const u8,
    id_len: u32,
    method: *const u8,
    method_len: u32,
    args_json: *const u8,
    args_len: u32,
    result_out: *mut u8,
    result_cap: u32,
) -> i32;
type FfiSetField = extern "C" fn(
    instance_id: *const u8,
    id_len: u32,
    name: *const u8,
    name_len: u32,
    value_json: *const u8,
    value_len: u32,
) -> i32;
type FfiGetField = extern "C" fn(
    instance_id: *const u8,
    id_len: u32,
    name: *const u8,
    name_len: u32,
    result_out: *mut u8,
    result_cap: u32,
) -> i32;
type FfiDestroyInstance = extern "C" fn(instance_id: *const u8, id_len: u32) -> i32;
type FfiShutdown = extern "C" fn() -> i32;

// ---------------------------------------------------------------------------
// Runtime handle
// ---------------------------------------------------------------------------

/// Opaque handle to the managed ILRuntime runtime.
struct ManagedRuntime {
    #[expect(dead_code)]
    lib_handle: *mut c_void,
    load_assembly: FfiLoadAssembly,
    instantiate: FfiInstantiate,
    call_method: FfiCallMethod,
    set_field: FfiSetField,
    get_field: FfiGetField,
    destroy_instance: FfiDestroyInstance,
    shutdown: FfiShutdown,
}

// SAFETY: The function pointers are loaded from a shared library that
// lives for the duration of the process. All calls are serialized
// through &mut self on ScriptHost.
unsafe impl Send for ManagedRuntime {}
unsafe impl Sync for ManagedRuntime {}

impl ManagedRuntime {
    /// Load the managed ILRuntime bridge DLL and resolve all FFI symbols.
    fn load(library_path: &str) -> Result<Self, ScriptError> {
        // In a real implementation, this would use `libloading` or similar
        // to load the managed runtime DLL and resolve symbols.
        //
        // For now, this returns an error — the actual loading requires the
        // managed C# runtime project to be built first.
        let _ = library_path;
        Err(ScriptError::HostError(
            "ILRuntime managed runtime not yet loaded (stub)".into(),
        ))
    }
}

impl Drop for ManagedRuntime {
    fn drop(&mut self) {
        let _ = (self.shutdown)();
        // TODO: unload library when libloading is integrated
    }
}

// ---------------------------------------------------------------------------
// ILRuntime script instance
// ---------------------------------------------------------------------------

/// A script instance hosted in the ILRuntime AppDomain.
pub struct ILRuntimeInstance {
    instance_id: String,
    runtime: *mut ManagedRuntime,
}

impl std::fmt::Debug for ILRuntimeInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ILRuntimeInstance")
            .field("instance_id", &self.instance_id)
            .finish()
    }
}

// SAFETY: The ManagedRuntime is Send+Sync. All operations go through FFI
// and are externally synchronized by ScriptHost's &mut self.
unsafe impl Send for ILRuntimeInstance {}

impl ScriptInstance for ILRuntimeInstance {
    fn call(&mut self, function: &str, args: &[ScriptValue]) -> Result<ScriptValue, ScriptError> {
        if self.runtime.is_null() {
            return Err(ScriptError::HostError("ILRuntime not initialized".into()));
        }
        let rt = unsafe { &*self.runtime };

        // Serialize args to JSON for the FFI call
        let args_json = serde_json::to_string(args)
            .map_err(|e| ScriptError::ExecutionError(format!("Failed to serialize args: {e}")))?;

        let mut result_buf = [0u8; 4096];
        let result_code = (rt.call_method)(
            self.instance_id.as_ptr(),
            self.instance_id.len() as u32,
            function.as_ptr(),
            function.len() as u32,
            args_json.as_ptr(),
            args_json.len() as u32,
            result_buf.as_mut_ptr(),
            result_buf.len() as u32,
        );

        if result_code != 0 {
            // Error — try to read the error message from the result buffer
            let msg = String::from_utf8_lossy(&result_buf)
                .trim_end_matches('\0')
                .to_string();
            return Err(ScriptError::ExecutionError(if msg.is_empty() {
                format!("CallMethod '{function}' failed (code {result_code})")
            } else {
                msg
            }));
        }

        // Parse result JSON back to ScriptValue
        let len = result_buf
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(result_buf.len());
        let result_str = std::str::from_utf8(&result_buf[..len])
            .map_err(|_| ScriptError::ExecutionError("Invalid UTF-8 in FFI result".into()))?;

        if result_str.is_empty() {
            return Ok(ScriptValue::Null);
        }

        serde_json::from_str(result_str)
            .map_err(|e| ScriptError::ExecutionError(format!("Failed to parse FFI result: {e}")))
    }

    fn set_field(&mut self, name: &str, value: ScriptValue) -> Result<(), ScriptError> {
        if self.runtime.is_null() {
            return Err(ScriptError::HostError("ILRuntime not initialized".into()));
        }
        let rt = unsafe { &*self.runtime };

        let value_json = serde_json::to_string(&value)
            .map_err(|e| ScriptError::ExecutionError(format!("Failed to serialize value: {e}")))?;

        let result_code = (rt.set_field)(
            self.instance_id.as_ptr(),
            self.instance_id.len() as u32,
            name.as_ptr(),
            name.len() as u32,
            value_json.as_ptr(),
            value_json.len() as u32,
        );

        if result_code != 0 {
            return Err(ScriptError::ExecutionError(format!(
                "SetField '{name}' failed (code {result_code})"
            )));
        }
        Ok(())
    }

    fn get_field(&self, name: &str) -> Option<ScriptValue> {
        if self.runtime.is_null() {
            return None;
        }
        let rt = unsafe { &*self.runtime };

        let mut result_buf = [0u8; 4096];
        let result_code = (rt.get_field)(
            self.instance_id.as_ptr(),
            self.instance_id.len() as u32,
            name.as_ptr(),
            name.len() as u32,
            result_buf.as_mut_ptr(),
            result_buf.len() as u32,
        );

        if result_code != 0 {
            return None;
        }

        let len = result_buf
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(result_buf.len());
        let result_str = std::str::from_utf8(&result_buf[..len]).ok()?;

        if result_str.is_empty() {
            return None;
        }

        serde_json::from_str(result_str).ok()
    }
}

// ---------------------------------------------------------------------------
// ILRuntime host
// ---------------------------------------------------------------------------

/// A [`ScriptHost`] that runs C# scripts through ILRuntime.
///
/// # Example
///
/// ```ignore
/// let mut host = ILRuntimeHost::new("ilruntime");
/// host.load_runtime("path/to/ILRuntime.Bridge.dll")?;
/// let handle = host.load_assembly("my-scripts", &dll_bytes)?;
/// let mut instance = host.instantiate(&handle)?;
/// let result = instance.call("OnUpdate", &[ScriptValue::Float(0.016)])?;
/// ```
pub struct ILRuntimeHost {
    name: String,
    runtime: Option<ManagedRuntime>,
    /// Track loaded assemblies for unload
    assemblies: Vec<(ScriptHandle, String)>,
    next_instance_id: u64,
    initialized: AtomicBool,
}

impl ILRuntimeHost {
    /// Create a new ILRuntime host with the given display name.
    ///
    /// Call [`load_runtime`](Self::load_runtime) to initialize the managed
    /// runtime before performing any script operations.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            runtime: None,
            assemblies: Vec::new(),
            next_instance_id: 0,
            initialized: AtomicBool::new(false),
        }
    }

    /// Load the managed ILRuntime bridge library.
    ///
    /// `library_path` is the path to the compiled .NET runtime DLL that
    /// hosts the ILRuntime AppDomain and exposes the FFI functions.
    pub fn load_runtime(&mut self, library_path: &str) -> Result<(), ScriptError> {
        let runtime = ManagedRuntime::load(library_path)?;
        self.initialized.store(true, Ordering::SeqCst);
        self.runtime = Some(runtime);
        Ok(())
    }

    /// Whether the managed runtime has been successfully loaded.
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
    }

    fn require_runtime(&self) -> Result<(), ScriptError> {
        if !self.is_initialized() {
            return Err(ScriptError::HostError(
                "ILRuntime runtime not loaded — call load_runtime() first".into(),
            ));
        }
        Ok(())
    }
}

impl ScriptHost for ILRuntimeHost {
    fn name(&self) -> &str {
        &self.name
    }

    fn load_assembly(
        &mut self,
        id: &str,
        assembly_data: &[u8],
    ) -> Result<ScriptHandle, ScriptError> {
        self.require_runtime()?;
        let rt = self.runtime.as_ref().unwrap();

        let result_code = (rt.load_assembly)(
            id.as_ptr(),
            id.len() as u32,
            assembly_data.as_ptr(),
            assembly_data.len() as u32,
        );

        if result_code != 0 {
            return Err(ScriptError::LoadFailed(format!(
                "ILRuntime load_assembly '{id}' failed (code {result_code})"
            )));
        }

        let handle = ScriptHandle::new(id);
        self.assemblies.push((handle.clone(), id.to_string()));
        Ok(handle)
    }

    fn instantiate(
        &mut self,
        handle: &ScriptHandle,
    ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
        self.require_runtime()?;
        let rt = self.runtime.as_ref().unwrap();

        let instance_id = format!("inst-{:04x}", self.next_instance_id);
        self.next_instance_id += 1;

        // Use assembly id as both assembly and class for now
        // (real impl would get class_name from ScriptComponent)
        let assembly_id = handle.id();
        let class_name = "ScriptType";

        let result_code = (rt.instantiate)(
            assembly_id.as_ptr(),
            assembly_id.len() as u32,
            class_name.as_ptr(),
            class_name.len() as u32,
            instance_id.as_ptr(),
            instance_id.len() as u32,
        );

        if result_code != 0 {
            return Err(ScriptError::LoadFailed(format!(
                "ILRuntime instantiate failed for '{assembly_id}:{class_name}' (code {result_code})"
            )));
        }

        // Store a raw pointer to the runtime so instances can call back
        let runtime_ptr: *mut ManagedRuntime = self
            .runtime
            .as_mut()
            .map(|r| r as *mut ManagedRuntime)
            .unwrap();

        Ok(Box::new(ILRuntimeInstance {
            instance_id,
            runtime: runtime_ptr,
        }))
    }

    fn unload(&mut self, handle: &ScriptHandle) -> Result<(), ScriptError> {
        if let Some(rt) = &self.runtime {
            let _ = (rt.destroy_instance)(handle.id().as_ptr(), handle.id().len() as u32);
        }
        self.assemblies.retain(|(h, _)| h.id() != handle.id());
        Ok(())
    }
}

impl Drop for ILRuntimeHost {
    fn drop(&mut self) {
        self.runtime.take();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ilruntime_host_name() {
        let host = ILRuntimeHost::new("test-host");
        assert_eq!(host.name(), "test-host");
    }

    #[test]
    fn ilruntime_host_not_initialized_by_default() {
        let host = ILRuntimeHost::new("test");
        assert!(!host.is_initialized());
    }

    #[test]
    fn ilruntime_host_load_before_initialized_fails() {
        let mut host = ILRuntimeHost::new("test");
        let result = host.load_assembly("asm", b"data");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not loaded"));
    }

    #[test]
    fn ilruntime_host_instantiate_before_initialized_fails() {
        let mut host = ILRuntimeHost::new("test");
        let handle = ScriptHandle::new("asm");
        let result = host.instantiate(&handle);
        assert!(result.is_err());
    }

    #[test]
    fn ilruntime_instance_debug() {
        let name = std::any::type_name::<ILRuntimeInstance>();
        assert!(name.contains("ILRuntimeInstance"));
    }

    #[test]
    fn ilruntime_instance_call_fails_without_runtime() {
        let mut inst = ILRuntimeInstance {
            instance_id: "test".into(),
            runtime: std::ptr::null_mut(),
        };
        let result = inst.call("Foo", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn ilruntime_instance_field_fails_without_runtime() {
        let inst = ILRuntimeInstance {
            instance_id: "test".into(),
            runtime: std::ptr::null_mut(),
        };
        assert!(inst.get_field("x").is_none());
    }
}

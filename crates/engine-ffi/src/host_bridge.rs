//! Installs the Rust host callback table into the C#-loaded `engine_ffi` cdylib.
//!
//! Cargo builds this crate as both an `rlib` and a `cdylib`. Those artifacts
//! contain independent Rust statics, so they must never pretend to share the
//! registry or active world. The host calls [`install_cdylib_registry`] with
//! its callback table; this module loads and keeps the exact cdylib alive, then
//! invokes its versioned `ffi_install_host_registry` export.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, OnceLock};

use libloading::Library;

use crate::registry::{
    FfiRegistry, FFI_INSTALL_ALREADY_INITIALIZED, FFI_INSTALL_NULL_REGISTRY, FFI_INSTALL_OK,
    FFI_INSTALL_PANICKED, FFI_INSTALL_SIZE_MISMATCH, FFI_INSTALL_VERSION_MISMATCH,
    FFI_REGISTRY_ABI_VERSION,
};

type AbiVersionFn = unsafe extern "C" fn() -> u32;
type StructSizeFn = unsafe extern "C" fn() -> u32;
type InstallRegistryFn = unsafe extern "C" fn(*const FfiRegistry, u32, u32) -> u32;

struct LoadedCdylib {
    _library: Library,
    path: PathBuf,
}

static LOADED_CDYLIB: OnceLock<LoadedCdylib> = OnceLock::new();
static INSTALL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// A concrete failure to locate, validate, or initialise the C# native DLL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostBridgeError {
    message: String,
}

impl HostBridgeError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for HostBridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HostBridgeError {}

/// Install a copy of the host callback table into the native DLL used by C#.
///
/// The DLL handle is retained for the process lifetime so every installed
/// callback remains callable and subsequent P/Invoke resolution reuses the
/// already-loaded module. A successful repeated call is a no-op.
pub fn install_cdylib_registry(host_registry: &FfiRegistry) -> Result<(), HostBridgeError> {
    let _guard = INSTALL_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if LOADED_CDYLIB.get().is_some() {
        return Ok(());
    }

    let (library, path) = load_cdylib()?;
    validate_and_install(&library, host_registry)?;

    let path = path.canonicalize().unwrap_or(path);
    // Managed DllImport resolution reads these values to load this exact
    // module. Child ProcessHost processes inherit the path but not this PID,
    // so Engine.API can reject accidental out-of-process direct P/Invoke.
    std::env::set_var("ENGINE_FFI_LIBRARY", &path);
    std::env::set_var("ENGINE_FFI_HOST_PID", std::process::id().to_string());

    LOADED_CDYLIB
        .set(LoadedCdylib {
            _library: library,
            path,
        })
        .map_err(|_| HostBridgeError::new("engine_ffi cdylib was installed concurrently"))?;
    Ok(())
}

/// Path of the native DLL retained by a successful bridge installation.
pub fn loaded_cdylib_path() -> Option<PathBuf> {
    LOADED_CDYLIB.get().map(|loaded| loaded.path.clone())
}

fn validate_and_install(
    library: &Library,
    host_registry: &FfiRegistry,
) -> Result<(), HostBridgeError> {
    // SAFETY: Symbol names and signatures are part of the versioned engine-ffi
    // ABI. The version and exact struct-size checks run before table install.
    unsafe {
        let abi_version = library
            .get::<AbiVersionFn>(b"ffi_registry_abi_version\0")
            .map_err(|error| {
                HostBridgeError::new(format!("missing ABI version export: {error}"))
            })?;
        let struct_size = library
            .get::<StructSizeFn>(b"ffi_registry_struct_size\0")
            .map_err(|error| HostBridgeError::new(format!("missing ABI size export: {error}")))?;
        let install = library
            .get::<InstallRegistryFn>(b"ffi_install_host_registry\0")
            .map_err(|error| {
                HostBridgeError::new(format!("missing registry install export: {error}"))
            })?;

        let dll_version = abi_version();
        if dll_version != FFI_REGISTRY_ABI_VERSION {
            return Err(HostBridgeError::new(format!(
                "engine_ffi ABI version mismatch: host={}, dll={dll_version}",
                FFI_REGISTRY_ABI_VERSION
            )));
        }

        let host_size = std::mem::size_of::<FfiRegistry>()
            .try_into()
            .map_err(|_| HostBridgeError::new("host FfiRegistry is larger than u32::MAX"))?;
        let dll_size = struct_size();
        if dll_size != host_size {
            return Err(HostBridgeError::new(format!(
                "engine_ffi registry size mismatch: host={host_size}, dll={dll_size}"
            )));
        }

        let status = install(host_registry, FFI_REGISTRY_ABI_VERSION, host_size);
        match status {
            FFI_INSTALL_OK => Ok(()),
            FFI_INSTALL_NULL_REGISTRY => {
                Err(HostBridgeError::new("cdylib rejected a null host registry"))
            }
            FFI_INSTALL_VERSION_MISMATCH => Err(HostBridgeError::new(
                "cdylib rejected the host registry ABI version",
            )),
            FFI_INSTALL_SIZE_MISMATCH => Err(HostBridgeError::new(
                "cdylib rejected the host registry byte size",
            )),
            FFI_INSTALL_ALREADY_INITIALIZED => Err(HostBridgeError::new(
                "cdylib registry was already initialized by another host",
            )),
            FFI_INSTALL_PANICKED => Err(HostBridgeError::new(
                "cdylib panicked while installing the host registry",
            )),
            other => Err(HostBridgeError::new(format!(
                "cdylib returned unknown registry install status {other}"
            ))),
        }
    }
}

fn load_cdylib() -> Result<(Library, PathBuf), HostBridgeError> {
    let candidates = cdylib_candidates();
    let mut failures = Vec::new();

    for path in candidates {
        // SAFETY: Loading a native library can run its initializers. Candidate
        // paths are restricted to an explicit override or the host executable's
        // own artifact directories, and the ABI is validated before use.
        match unsafe { Library::new(&path) } {
            Ok(library) => return Ok((library, path)),
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }

    Err(HostBridgeError::new(format!(
        "unable to load engine_ffi cdylib; tried {}",
        failures.join("; ")
    )))
}

fn cdylib_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("ENGINE_FFI_LIBRARY") {
        push_unique(&mut candidates, PathBuf::from(path));
    }

    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            push_unique(&mut candidates, directory.join(cdylib_filename()));
            // Cargo test executables live under target/<profile>/deps while
            // the distributable cdylib also lives one directory above.
            if directory.file_name().is_some_and(|name| name == "deps") {
                if let Some(profile_directory) = directory.parent() {
                    push_unique(&mut candidates, profile_directory.join(cdylib_filename()));
                }
            }
        }
    }

    candidates
}

fn cdylib_filename() -> String {
    format!(
        "{}engine_ffi{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

fn push_unique(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|path| same_path(path, &candidate)) {
        paths.push(candidate);
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        FfiAsyncCallback, FfiAsyncHandle, FfiComponentTypeId, FfiCoroutineHandle, FfiEntityId,
        FfiManagedCoroutineDescriptor,
    };

    type EntitySpawnFn = unsafe extern "C" fn() -> FfiEntityId;
    type EntityAliveFn = unsafe extern "C" fn(FfiEntityId) -> bool;
    type ComponentTypeIdFn = unsafe extern "C" fn(*const std::ffi::c_char) -> FfiComponentTypeId;
    type ComponentTypeCountFn = unsafe extern "C" fn() -> u32;

    const EXPECTED_ENTITY: FfiEntityId = FfiEntityId {
        index: 77,
        generation: 9,
    };

    extern "C" fn entity_spawn() -> FfiEntityId {
        EXPECTED_ENTITY
    }
    extern "C" fn entity_bool(entity: FfiEntityId) -> bool {
        entity == EXPECTED_ENTITY
    }
    extern "C" fn component_type_id(name: *const std::ffi::c_char) -> FfiComponentTypeId {
        if name.is_null() {
            FfiComponentTypeId::INVALID
        } else {
            FfiComponentTypeId(123)
        }
    }
    extern "C" fn component_type_count() -> u32 {
        1
    }
    extern "C" fn component_get(
        _entity: FfiEntityId,
        _type_id: FfiComponentTypeId,
        out_len: &mut u32,
    ) -> *mut u8 {
        *out_len = 0;
        std::ptr::null_mut()
    }
    extern "C" fn component_set(
        _entity: FfiEntityId,
        _type_id: FfiComponentTypeId,
        _data: *const u8,
        _len: u32,
    ) -> bool {
        false
    }
    unsafe extern "C" fn coroutine_start(
        _descriptor: *const FfiManagedCoroutineDescriptor,
    ) -> FfiCoroutineHandle {
        FfiCoroutineHandle::INVALID
    }
    extern "C" fn coroutine_cancel(_handle: FfiCoroutineHandle) {}
    extern "C" fn async_complete(_handle: FfiAsyncHandle) -> bool {
        false
    }
    extern "C" fn async_start(
        _url: *const std::ffi::c_char,
        _callback: FfiAsyncCallback,
        _user_data: u64,
    ) -> FfiAsyncHandle {
        FfiAsyncHandle(0)
    }
    extern "C" fn dispatch() {}

    fn test_registry() -> FfiRegistry {
        FfiRegistry {
            entity_spawn,
            entity_destroy: entity_bool,
            entity_is_alive: entity_bool,
            component_type_id,
            component_type_count,
            component_get_ptr: component_get,
            component_set_ptr: component_set,
            coroutine_start,
            coroutine_cancel,
            async_is_complete: async_complete,
            async_load_image: async_start,
            async_http_get: async_start,
            dispatch_main_thread_callbacks: dispatch,
        }
    }

    #[test]
    fn dynamically_loaded_cdylib_calls_back_into_host_registry() {
        install_cdylib_registry(&test_registry()).expect("install host registry in cdylib");
        let loaded = LOADED_CDYLIB.get().expect("cdylib retained");

        // SAFETY: The symbols are exported by the ABI-validated cdylib and use
        // the declared C signatures. Calls occur while `loaded._library` lives.
        unsafe {
            let spawn = loaded
                ._library
                .get::<EntitySpawnFn>(b"ffi_entity_spawn\0")
                .expect("ffi_entity_spawn export");
            let is_alive = loaded
                ._library
                .get::<EntityAliveFn>(b"ffi_entity_is_alive\0")
                .expect("ffi_entity_is_alive export");
            let component_type_id = loaded
                ._library
                .get::<ComponentTypeIdFn>(b"ffi_component_type_id\0")
                .expect("ffi_component_type_id export");
            let component_type_count = loaded
                ._library
                .get::<ComponentTypeCountFn>(b"ffi_component_type_count\0")
                .expect("ffi_component_type_count export");

            let entity = spawn();
            assert_eq!(entity, EXPECTED_ENTITY);
            assert!(is_alive(entity));
            assert!(!is_alive(FfiEntityId::INVALID));

            let component_name = std::ffi::CString::new("test.component").unwrap();
            assert_eq!(
                component_type_id(component_name.as_ptr()),
                FfiComponentTypeId(123)
            );
            assert_eq!(component_type_count(), 1);
        }
        assert!(loaded_cdylib_path().is_some_and(|path| path.exists()));
    }
}

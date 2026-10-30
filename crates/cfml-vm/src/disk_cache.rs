//! Persistent on-disk bytecode cache.
//!
//! The in-memory `BytecodeCache` is kept — the disk cache only replaces the
//! compile step when a source file is not in memory. The load path is:
//!
//! 1. Check in-memory cache (with freshness check in dev mode)
//! 2. If miss, check disk cache (content-hash based key)
//! 3. If miss, compile from source and write to disk (best effort)
//!
//! ## Key format
//!
//! Each cache entry is keyed by:
//! - SHA-256 hash of source bytes
//! - Engine version (CARGO_PKG_VERSION)
//! - Bytecode format version (bumped when BytecodeOp or codegen changes)
//!
//! A key mismatch is always a cache miss, never an error.
//!
//! ## Serialization Strategy
//!
//! Only stable data is serialized. OnceLock memos are rebuilt on first use
//! after deserialization. Function IDs are remapped on load.
//!
//! ## Trust
//!
//! The cache directory is treated as code. It must not be writable by
//! untrusted users. Each entry includes a checksum so truncated/corrupt
//! files are misses, not crashes.

use cfml_codegen::compiler::{BytecodeFunction, BytecodeOp, BytecodeProgram};
use cfml_common::dynamic::CfmlAccess;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Bytecode format version. MUST be bumped whenever `BytecodeOp`, codegen
/// output, or any serialized structure changes in a way that would make old
/// cache entries incompatible.
///
/// History:
/// - 1: Initial disk cache implementation (v0.654.0, bincode 1.3.3)
/// - 2: Switched to bincode 2.0.1 (v0.742.2)
/// - 3: `BytecodeOp::TakeProperty` added after `TakeSlot`, shifting the
///   variant indices of every later op (serde encodes enums by index)
pub const BYTECODE_FORMAT_VERSION: u32 = 3;

/// Compute cache key for a source file, optionally including the source path.
/// The path component ensures that the same file compiled via different paths
/// (e.g., physical path vs mapping-qualified path) produces distinct cache entries.
fn compute_cache_key(source_bytes: &[u8], source_file: Option<&str>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(source_bytes);
    if let Some(path) = source_file {
        hasher.update(path.as_bytes());
    }
    hasher.finalize().into()
}

/// Compute checksum for serialized data (simple FNV-1a)
fn compute_checksum(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Build cache file path from source hash
fn cache_file_path(cache_dir: &Path, source_hash: &[u8; 32]) -> PathBuf {
    let hex = source_hash
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    cache_dir.join(format!("{}.bc", hex))
}

/// Metadata header for cache file
#[derive(Debug, Serialize, Deserialize)]
struct CacheHeader {
    engine_version: String,
    format_version: u32,
    source_hash: [u8; 32],
    checksum: u64,
}

/// Serializable representation of CfmlValue (for param_defaults)
#[derive(Debug, Serialize, Deserialize)]
enum SerializableCfmlValue {
    Null,
    Bool(bool),
    Int(i64),
    Double(f64),
    String(String),
    Other, // For complex values, we just mark them as "other" and reconstruct as Null
}

impl From<&cfml_common::dynamic::CfmlValue> for SerializableCfmlValue {
    fn from(v: &cfml_common::dynamic::CfmlValue) -> Self {
        use cfml_common::dynamic::CfmlValue;
        match v {
            CfmlValue::Null => SerializableCfmlValue::Null,
            CfmlValue::Bool(b) => SerializableCfmlValue::Bool(*b),
            CfmlValue::Int(n) => SerializableCfmlValue::Int(*n),
            CfmlValue::Double(f) => SerializableCfmlValue::Double(*f),
            CfmlValue::String(s) => SerializableCfmlValue::String((**s).clone()),
            _ => SerializableCfmlValue::Other,
        }
    }
}

impl From<&SerializableCfmlValue> for cfml_common::dynamic::CfmlValue {
    fn from(v: &SerializableCfmlValue) -> Self {
        use cfml_common::dynamic::CfmlValue;
        match v {
            SerializableCfmlValue::Null => CfmlValue::Null,
            SerializableCfmlValue::Bool(b) => CfmlValue::Bool(*b),
            SerializableCfmlValue::Int(n) => CfmlValue::Int(*n),
            SerializableCfmlValue::Double(f) => CfmlValue::Double(*f),
            SerializableCfmlValue::String(s) => CfmlValue::String(Arc::new(s.clone())),
            SerializableCfmlValue::Other => CfmlValue::Null,
        }
    }
}

/// Serializable function with only stable data (no OnceLock fields)
#[derive(Debug, Serialize, Deserialize)]
struct SerializableFunction {
    name: String,
    params: Vec<String>,
    required_params: Vec<bool>,
    has_default: Vec<bool>,
    /// For now, we serialize instructions as raw bincode bytes
    /// This is a pragmatic approach - if BytecodeOp changes incompatibly,
    /// deserialization will fail and we'll get a cache miss.
    instructions_blob: Vec<u8>,
    source_file: Option<String>,
    global_id: u32, // Will be remapped on load
    declared_local_mode: Option<bool>,
    param_types: Vec<Option<String>>,
    return_type: Option<String>,
    param_annotations: Vec<Vec<(String, String)>>,
    param_defaults: Vec<Option<SerializableCfmlValue>>,
    is_component_method: bool,
    access: CfmlAccess,
    metadata: Vec<(String, String)>,
    is_template_frame: bool,
    is_generated_accessor: bool,
    output_suppressed: bool,
    chain_tier: u8,
    slot_names: Vec<String>,
}

/// Convert BytecodeFunction to serializable form
fn serialize_function(func: &BytecodeFunction) -> Result<SerializableFunction, Box<dyn std::error::Error>> {
    let instructions_blob = bincode::serde::encode_to_vec(&func.instructions, bincode::config::standard())?;
    
    Ok(SerializableFunction {
        name: func.name.clone(),
        params: func.params.clone(),
        required_params: func.required_params.clone(),
        has_default: func.has_default.clone(),
        instructions_blob,
        source_file: func.source_file.clone(),
        global_id: func.global_id,
        declared_local_mode: func.declared_local_mode,
        param_types: func.param_types.clone(),
        return_type: func.return_type.clone(),
        param_annotations: func.param_annotations.clone(),
        param_defaults: func.param_defaults.iter().map(|o| o.as_ref().map(SerializableCfmlValue::from)).collect(),
        is_component_method: func.is_component_method,
        access: func.access.clone(),
        metadata: func.metadata.clone(),
        is_template_frame: func.is_template_frame,
        is_generated_accessor: func.is_generated_accessor,
        output_suppressed: func.output_suppressed,
        chain_tier: func.chain_tier,
        slot_names: func.slot_names.iter().map(|n| n.as_str().to_string()).collect(),
    })
}

/// Remap function IDs in a single bytecode instruction
fn remap_instruction_ids(op: &mut BytecodeOp, id_remap: &HashMap<u32, u32>) {
    match op {
        BytecodeOp::DefineFunction(ref mut gid) => {
            if let Some(&new_id) = id_remap.get(&(*gid as u32)) {
                *gid = new_id as usize;
            }
        }
        BytecodeOp::DefineComponentMethods(ref mut methods_box) => {
            // ComponentMethods is in a Box, so we need to remap its gids
            for gid in &mut methods_box.gids {
                if let Some(&new_id) = id_remap.get(&(*gid as u32)) {
                    *gid = new_id as usize;
                }
            }
        }
        _ => {
            // Other ops don't contain function IDs
        }
    }
}

/// Convert serializable form back to BytecodeFunction, remapping function IDs
fn deserialize_function(
    ser: &SerializableFunction,
    new_global_id: u32,
    id_remap: &HashMap<u32, u32>,
) -> Result<BytecodeFunction, Box<dyn std::error::Error>> {
    let mut instructions: Vec<BytecodeOp> = bincode::serde::decode_from_slice(&ser.instructions_blob, bincode::config::standard())?.0;
    
    // Remap all function IDs in instructions
    for instruction in &mut instructions {
        remap_instruction_ids(instruction, id_remap);
    }
    
    Ok(BytecodeFunction {
        name: ser.name.clone(),
        params: ser.params.clone(),
        param_keys: std::sync::OnceLock::new(), // Rebuilt on first use
        name_shared: std::sync::OnceLock::new(), // Rebuilt on first use
        source_file_shared: std::sync::OnceLock::new(), // Rebuilt on first use
        args_needed: std::sync::OnceLock::new(), // Computed lazily
        args_never_escapes: std::sync::OnceLock::new(), // Computed lazily
        rebinds_param: std::sync::OnceLock::new(), // Computed lazily
        local_may_escape: std::sync::OnceLock::new(), // Computed lazily
        params_marker: std::sync::OnceLock::new(), // Computed lazily
        cfc_body: std::sync::OnceLock::new(), // Computed lazily
        source_file_value: std::sync::OnceLock::new(), // Computed lazily
        required_params: ser.required_params.clone(),
        has_default: ser.has_default.clone(),
        instructions,
        source_file: ser.source_file.clone(),
        global_id: new_global_id,
        declared_local_mode: ser.declared_local_mode,
        param_types: ser.param_types.clone(),
        return_type: ser.return_type.clone(),
        param_annotations: ser.param_annotations.clone(),
        param_defaults: ser.param_defaults.iter().map(|o| o.as_ref().map(|v| v.into())).collect(),
        is_component_method: ser.is_component_method,
        access: ser.access.clone(),
        metadata: ser.metadata.clone(),
        is_template_frame: ser.is_template_frame,
        is_generated_accessor: ser.is_generated_accessor,
        output_suppressed: ser.output_suppressed,
        chain_tier: ser.chain_tier,
        slot_names: ser.slot_names.iter().map(|s| cfml_common::name::Name::intern(s)).collect(),
    })
}

/// Serialized program
#[derive(Debug, Serialize, Deserialize)]
struct SerializedProgram {
    header: CacheHeader,
    functions: Vec<SerializableFunction>,
}

/// Serialize and write a program to the disk cache
pub fn write_to_disk_cache(
    cache_dir: &Path,
    source_bytes: &[u8],
    source_file: Option<&str>,
    program: &BytecodeProgram,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create cache directory if it doesn't exist
    std::fs::create_dir_all(cache_dir)?;

    let source_hash = compute_cache_key(source_bytes, source_file);
    let cache_file = cache_file_path(cache_dir, &source_hash);

    // Serialize all functions
    let functions: Result<Vec<_>, _> = program
        .functions
        .iter()
        .map(|f| serialize_function(f))
        .collect();
    
    let functions = match functions {
        Ok(f) => f,
        Err(_) => return Ok(()), // Silently skip if serialization fails
    };

    // Build the cache entry
    let mut serialized = SerializedProgram {
        header: CacheHeader {
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
            format_version: BYTECODE_FORMAT_VERSION,
            source_hash,
            checksum: 0, // Will be computed below
        },
        functions,
    };

    // Serialize without checksum first
    let mut data = match bincode::serde::encode_to_vec(&serialized, bincode::config::standard()) {
        Ok(d) => d,
        Err(_) => return Ok(()), // Silently skip
    };
    
    // Compute checksum and add it
    serialized.header.checksum = compute_checksum(&data);
    data = bincode::serde::encode_to_vec(&serialized, bincode::config::standard())?;

    // Atomic write: a temp file unique to THIS writer, then rename. A cold
    // server compiles the same template on several request threads at once
    // (and a `--warm` can overlap a running server); a temp name shared by
    // all of them lets one writer truncate another's half-written file and
    // rename a torn entry into place. The checksum would catch it on read,
    // but at the price of a miss and a recompile every time.
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_file = cache_file.with_extension(format!("{}.{}.tmp", std::process::id(), seq));
    if let Err(e) = std::fs::write(&temp_file, &data).and_then(|_| std::fs::rename(&temp_file, &cache_file)) {
        let _ = std::fs::remove_file(&temp_file);
        return Err(e.into());
    }

    Ok(())
}

/// Read and deserialize a program from the disk cache
pub fn read_from_disk_cache(
    cache_dir: &Path,
    source_bytes: &[u8],
    source_file: Option<&str>,
) -> Result<Option<BytecodeProgram>, Box<dyn std::error::Error>> {
    let source_hash = compute_cache_key(source_bytes, source_file);
    let cache_file = cache_file_path(cache_dir, &source_hash);

    // File doesn't exist → cache miss
    if !cache_file.exists() {
        return Ok(None);
    }

    // Read the file (failures are cache misses, not errors)
    let data = match std::fs::read(&cache_file) {
        Ok(d) => d,
        Err(_) => return Ok(None),
    };

    // Deserialize the cache entry
    let mut serialized: SerializedProgram = match bincode::serde::decode_from_slice(&data, bincode::config::standard()) {
        Ok((s, _)) => s,
        Err(_) => return Ok(None), // Corrupt or incompatible → miss
    };

    // Verify metadata
    if serialized.header.engine_version != env!("CARGO_PKG_VERSION") {
        return Ok(None); // Version mismatch → miss
    }

    if serialized.header.format_version != BYTECODE_FORMAT_VERSION {
        return Ok(None); // Format version mismatch → miss
    }

    if serialized.header.source_hash != source_hash {
        return Ok(None); // Source hash mismatch → miss
    }

    // Verify checksum
    let stored_checksum = serialized.header.checksum;
    serialized.header.checksum = 0;
    let recomputed_data = bincode::serde::encode_to_vec(&serialized, bincode::config::standard())?;
    let computed_checksum = compute_checksum(&recomputed_data);
    
    if stored_checksum != computed_checksum {
        return Ok(None); // Checksum mismatch → corrupt file → miss
    }

    // Allocate fresh global IDs for all functions
    let mut id_remap: HashMap<u32, u32> = HashMap::new();
    for func in &serialized.functions {
        let new_id = cfml_codegen::compiler::next_global_fn_id();
        id_remap.insert(func.global_id, new_id);
    }

    // Deserialize functions with remapped IDs
    let functions: Result<Vec<_>, _> = serialized
        .functions
        .iter()
        .map(|sf| {
            let new_id = id_remap.get(&sf.global_id).copied().unwrap_or(sf.global_id);
            deserialize_function(sf, new_id, &id_remap).map(Arc::new)
        })
        .collect();

    let functions = match functions {
        Ok(f) => f,
        Err(_) => return Ok(None), // Deserialization failed → miss
    };

    Ok(Some(BytecodeProgram { functions }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfml_common::dynamic::CfmlAccess;
    use tempfile::TempDir;

    #[test]
    fn test_cache_key_stability() {
        let source1 = b"function test() { return 42; }";
        let source2 = b"function test() { return 42; }";
        let source3 = b"function test() { return 43; }";

        let hash1 = compute_cache_key(source1, None);
        let hash2 = compute_cache_key(source2, None);
        let hash3 = compute_cache_key(source3, None);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_write_and_read_cache() {
        let temp_dir = TempDir::new().unwrap();
        let cache_dir = temp_dir.path();

        let source = b"function test() { return 42; }";
        let program = BytecodeProgram {
            functions: vec![Arc::new(BytecodeFunction {
                name: "test".to_string(),
                params: vec![],
                param_keys: std::sync::OnceLock::new(),
                name_shared: std::sync::OnceLock::new(),
                source_file_shared: std::sync::OnceLock::new(),
                args_needed: std::sync::OnceLock::new(),
                args_never_escapes: std::sync::OnceLock::new(),
                rebinds_param: std::sync::OnceLock::new(),
                local_may_escape: std::sync::OnceLock::new(),
                params_marker: std::sync::OnceLock::new(),
                cfc_body: std::sync::OnceLock::new(),
                source_file_value: std::sync::OnceLock::new(),
                required_params: vec![],
                has_default: vec![],
                instructions: vec![
                    BytecodeOp::Integer(42),
                    BytecodeOp::Return,
                ],
                source_file: None,
                global_id: cfml_codegen::compiler::next_global_fn_id(),
                declared_local_mode: None,
                param_types: vec![],
                return_type: None,
                param_annotations: vec![],
                param_defaults: vec![],
                is_component_method: false,
                access: CfmlAccess::Public,
                metadata: vec![],
                is_template_frame: false,
                is_generated_accessor: false,
                output_suppressed: false,
                chain_tier: 0,
                slot_names: vec![],
            })],
        };

        write_to_disk_cache(cache_dir, source, None, &program).unwrap();

        let loaded = read_from_disk_cache(cache_dir, source, None).unwrap();
        assert!(loaded.is_some());

        let loaded_program = loaded.unwrap();
        assert_eq!(loaded_program.functions.len(), 1);
        assert_eq!(loaded_program.functions[0].name, "test");
        assert_eq!(loaded_program.functions[0].instructions.len(), 2);
    }

    #[test]
    fn test_cache_miss_on_version_mismatch() {
        let temp_dir = TempDir::new().unwrap();
        let cache_dir = temp_dir.path();
        let source = b"function test() { return 42; }";

        let source_hash = compute_cache_key(source, None);
        let cache_file = cache_file_path(cache_dir, &source_hash);
        std::fs::create_dir_all(cache_dir).unwrap();

        // Write a cache entry with a mismatched version
        let bad_entry = SerializedProgram {
            header: CacheHeader {
                engine_version: "999.999.999".to_string(),
                format_version: BYTECODE_FORMAT_VERSION,
                source_hash,
                checksum: 0,
            },
            functions: vec![],
        };

        let data = bincode::serde::encode_to_vec(&bad_entry, bincode::config::standard()).unwrap();
        std::fs::write(&cache_file, &data).unwrap();

        // Should be a cache miss
        let loaded = read_from_disk_cache(cache_dir, source, None).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_cache_miss_on_format_version_mismatch() {
        let temp_dir = TempDir::new().unwrap();
        let cache_dir = temp_dir.path();
        let source = b"function test() { return 42; }";

        let source_hash = compute_cache_key(source, None);
        let cache_file = cache_file_path(cache_dir, &source_hash);
        std::fs::create_dir_all(cache_dir).unwrap();

        // Write a cache entry with a mismatched format version
        let bad_entry = SerializedProgram {
            header: CacheHeader {
                engine_version: env!("CARGO_PKG_VERSION").to_string(),
                format_version: 999,
                source_hash,
                checksum: 0,
            },
            functions: vec![],
        };

        let data = bincode::serde::encode_to_vec(&bad_entry, bincode::config::standard()).unwrap();
        std::fs::write(&cache_file, &data).unwrap();

        // Should be a cache miss
        let loaded = read_from_disk_cache(cache_dir, source, None).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_function_id_remapping_comprehensive() {
        use cfml_codegen::compiler::{ComponentMethods, Name};
        
        let temp_dir = TempDir::new().unwrap();
        let cache_dir = temp_dir.path();
        let source = b"component { function init() { function inner() { return 1; } return this; } }";

        // Allocate some function IDs so the remapped IDs will definitely be different
        let _dummy1 = cfml_codegen::compiler::next_global_fn_id();
        let _dummy2 = cfml_codegen::compiler::next_global_fn_id();

        // Create functions with DefineFunction and DefineComponentMethods ops
        let inner_id = cfml_codegen::compiler::next_global_fn_id();
        let init_id = cfml_codegen::compiler::next_global_fn_id();
        let main_id = cfml_codegen::compiler::next_global_fn_id();

        let inner_func = Arc::new(BytecodeFunction {
            name: "inner".to_string(),
            params: vec![],
            param_keys: std::sync::OnceLock::new(),
            name_shared: std::sync::OnceLock::new(),
            source_file_shared: std::sync::OnceLock::new(),
            args_needed: std::sync::OnceLock::new(),
            args_never_escapes: std::sync::OnceLock::new(),
            rebinds_param: std::sync::OnceLock::new(),
            local_may_escape: std::sync::OnceLock::new(),
            params_marker: std::sync::OnceLock::new(),
            cfc_body: std::sync::OnceLock::new(),
            source_file_value: std::sync::OnceLock::new(),
            required_params: vec![],
            has_default: vec![],
            instructions: vec![
                BytecodeOp::Integer(1),
                BytecodeOp::Return,
            ],
            source_file: None,
            global_id: inner_id,
            declared_local_mode: None,
            param_types: vec![],
            return_type: None,
            param_annotations: vec![],
            param_defaults: vec![],
            is_component_method: true,
            access: CfmlAccess::Public,
            metadata: vec![],
            is_template_frame: false,
            is_generated_accessor: false,
            output_suppressed: false,
            chain_tier: 0,
            slot_names: vec![],
        });

        let init_func = Arc::new(BytecodeFunction {
            name: "init".to_string(),
            params: vec![],
            param_keys: std::sync::OnceLock::new(),
            name_shared: std::sync::OnceLock::new(),
            source_file_shared: std::sync::OnceLock::new(),
            args_needed: std::sync::OnceLock::new(),
            args_never_escapes: std::sync::OnceLock::new(),
            rebinds_param: std::sync::OnceLock::new(),
            local_may_escape: std::sync::OnceLock::new(),
            params_marker: std::sync::OnceLock::new(),
            cfc_body: std::sync::OnceLock::new(),
            source_file_value: std::sync::OnceLock::new(),
            required_params: vec![],
            has_default: vec![],
            instructions: vec![
                // DefineFunction should reference the inner function
                BytecodeOp::DefineFunction(inner_id as usize),
                BytecodeOp::StoreLocal(Name::intern("inner")),
                BytecodeOp::LoadLocal(Name::intern("this")),
                BytecodeOp::Return,
            ],
            source_file: None,
            global_id: init_id,
            declared_local_mode: None,
            param_types: vec![],
            return_type: None,
            param_annotations: vec![],
            param_defaults: vec![],
            is_component_method: true,
            access: CfmlAccess::Public,
            metadata: vec![],
            is_template_frame: false,
            is_generated_accessor: false,
            output_suppressed: false,
            chain_tier: 0,
            slot_names: vec![],
        });

        let main_func = Arc::new(BytecodeFunction {
            name: "__main__".to_string(),
            params: vec![],
            param_keys: std::sync::OnceLock::new(),
            name_shared: std::sync::OnceLock::new(),
            source_file_shared: std::sync::OnceLock::new(),
            args_needed: std::sync::OnceLock::new(),
            args_never_escapes: std::sync::OnceLock::new(),
            rebinds_param: std::sync::OnceLock::new(),
            local_may_escape: std::sync::OnceLock::new(),
            params_marker: std::sync::OnceLock::new(),
            cfc_body: std::sync::OnceLock::new(),
            source_file_value: std::sync::OnceLock::new(),
            required_params: vec![],
            has_default: vec![],
            instructions: vec![
                // DefineComponentMethods should reference both functions
                BytecodeOp::DefineComponentMethods(Box::new(ComponentMethods {
                    holder: Name::intern("__cfc"),
                    gids: vec![init_id as usize, inner_id as usize],
                })),
                BytecodeOp::Return,
            ],
            source_file: None,
            global_id: main_id,
            declared_local_mode: None,
            param_types: vec![],
            return_type: None,
            param_annotations: vec![],
            param_defaults: vec![],
            is_component_method: false,
            access: CfmlAccess::Public,
            metadata: vec![],
            is_template_frame: true,
            is_generated_accessor: false,
            output_suppressed: false,
            chain_tier: 0,
            slot_names: vec![],
        });

        let program = BytecodeProgram {
            functions: vec![main_func, init_func, inner_func],
        };

        // Write to cache
        write_to_disk_cache(cache_dir, source, None, &program).unwrap();

        // Allocate more dummy IDs to ensure remapped IDs are fresh
        let _dummy3 = cfml_codegen::compiler::next_global_fn_id();
        let _dummy4 = cfml_codegen::compiler::next_global_fn_id();

        // Load from cache
        let loaded = read_from_disk_cache(cache_dir, source, None).unwrap();
        assert!(loaded.is_some());

        let loaded_program = loaded.unwrap();
        assert_eq!(loaded_program.functions.len(), 3);

        // Verify function IDs were remapped to fresh values
        let loaded_main = &loaded_program.functions[0];
        let loaded_init = &loaded_program.functions[1];
        let loaded_inner = &loaded_program.functions[2];

        // IDs should be different from original
        assert_ne!(loaded_main.global_id, main_id);
        assert_ne!(loaded_init.global_id, init_id);
        assert_ne!(loaded_inner.global_id, inner_id);

        // Verify DefineFunction in init references the correct remapped inner ID
        if let BytecodeOp::DefineFunction(ref_id) = loaded_init.instructions[0] {
            assert_eq!(ref_id as u32, loaded_inner.global_id);
        } else {
            panic!("Expected DefineFunction op");
        }

        // Verify DefineComponentMethods in main references correct remapped IDs
        if let BytecodeOp::DefineComponentMethods(ref methods_box) = loaded_main.instructions[0] {
            assert_eq!(methods_box.gids.len(), 2);
            assert_eq!(methods_box.gids[0] as u32, loaded_init.global_id);
            assert_eq!(methods_box.gids[1] as u32, loaded_inner.global_id);
        } else {
            panic!("Expected DefineComponentMethods op");
        }
    }

    #[test]
    fn test_cache_key_includes_source_path() {
        let source = b"component { function test() { return 42; } }";
        
        let hash_no_path = compute_cache_key(source, None);
        let hash_path1 = compute_cache_key(source, Some("tests/oop/Component.cfc"));
        let hash_path2 = compute_cache_key(source, Some("/dotdotprobe/Component.cfc"));
        
        assert_ne!(hash_no_path, hash_path1, "Cache key should differ when path is included");
        assert_ne!(hash_path1, hash_path2, "Cache keys should differ for different paths");
        
        let hash_path1_again = compute_cache_key(source, Some("tests/oop/Component.cfc"));
        assert_eq!(hash_path1, hash_path1_again, "Cache key should be stable for same path");
    }
}

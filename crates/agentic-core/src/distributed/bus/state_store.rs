//! State Store - NATS KV Store for sharing validated code artifacts.
//!
//! Enables executors to share:
//! - Validated type signatures
//! - Function signatures
//! - Field definitions
//! - Import statements
//!
//! This prevents type mismatches (E0308) by letting executors
//! reference already-validated definitions.

use async_nats::jetstream::kv::{self, Store};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::nats_client::NatsClient;
use super::types::BusError;

/// Bucket names for the KV store.
pub const BUCKET_TYPES: &str = "AGENTIC_TYPES";
pub const BUCKET_FUNCTIONS: &str = "AGENTIC_FUNCTIONS";
pub const BUCKET_FIELDS: &str = "AGENTIC_FIELDS";
pub const BUCKET_IMPORTS: &str = "AGENTIC_IMPORTS";
pub const BUCKET_PATTERNS: &str = "AGENTIC_PATTERNS";

/// A validated type definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeDef {
    /// Type name (e.g., "TaskId", "TaskDef")
    pub name: String,
    /// Full definition (e.g., "pub struct TaskId(u64);")
    pub definition: String,
    /// Fields if struct (field_name -> type)
    pub fields: HashMap<String, String>,
    /// Variants if enum
    pub variants: Vec<String>,
    /// Derives
    pub derives: Vec<String>,
    /// Validated at timestamp
    pub validated_at: chrono::DateTime<chrono::Utc>,
}

impl TypeDef {
    pub fn new(name: impl Into<String>, definition: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            definition: definition.into(),
            fields: HashMap::new(),
            variants: vec![],
            derives: vec![],
            validated_at: chrono::Utc::now(),
        }
    }

    pub fn with_fields(mut self, fields: HashMap<String, String>) -> Self {
        self.fields = fields;
        self
    }

    pub fn with_variants(mut self, variants: Vec<String>) -> Self {
        self.variants = variants;
        self
    }
}

/// A validated function signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSig {
    /// Function name
    pub name: String,
    /// Full signature (e.g., "pub fn new(max_concurrent: usize) -> Self")
    pub signature: String,
    /// Parameters (name -> type)
    pub params: HashMap<String, String>,
    /// Return type
    pub return_type: Option<String>,
    /// Is async
    pub is_async: bool,
    /// Owner type (if method)
    pub impl_for: Option<String>,
    /// Validated at
    pub validated_at: chrono::DateTime<chrono::Utc>,
}

impl FunctionSig {
    pub fn new(name: impl Into<String>, signature: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            signature: signature.into(),
            params: HashMap::new(),
            return_type: None,
            is_async: false,
            impl_for: None,
            validated_at: chrono::Utc::now(),
        }
    }

    pub fn with_params(mut self, params: HashMap<String, String>) -> Self {
        self.params = params;
        self
    }

    pub fn with_return_type(mut self, ret: impl Into<String>) -> Self {
        self.return_type = Some(ret.into());
        self
    }

    pub fn async_fn(mut self) -> Self {
        self.is_async = true;
        self
    }

    pub fn method_of(mut self, type_name: impl Into<String>) -> Self {
        self.impl_for = Some(type_name.into());
        self
    }
}

/// State store for sharing validated artifacts via NATS KV.
pub struct StateStore {
    nats: Arc<NatsClient>,
    /// Local cache for fast lookups
    types_cache: Arc<RwLock<HashMap<String, TypeDef>>>,
    functions_cache: Arc<RwLock<HashMap<String, FunctionSig>>>,
    imports_cache: Arc<RwLock<Vec<String>>>,
}

impl StateStore {
    /// Create a new StateStore.
    pub async fn new(nats: Arc<NatsClient>) -> Result<Self, BusError> {
        let store = Self {
            nats,
            types_cache: Arc::new(RwLock::new(HashMap::new())),
            functions_cache: Arc::new(RwLock::new(HashMap::new())),
            imports_cache: Arc::new(RwLock::new(Vec::new())),
        };

        // Create KV buckets
        store.ensure_buckets().await?;

        Ok(store)
    }

    /// Ensure KV buckets exist.
    async fn ensure_buckets(&self) -> Result<(), BusError> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        for bucket_name in [
            BUCKET_TYPES,
            BUCKET_FUNCTIONS,
            BUCKET_FIELDS,
            BUCKET_IMPORTS,
            BUCKET_PATTERNS,
        ] {
            match js.get_key_value(bucket_name).await {
                Ok(_) => {
                    debug!("Using existing KV bucket: {}", bucket_name);
                }
                Err(_) => {
                    info!("Creating KV bucket: {}", bucket_name);
                    js.create_key_value(kv::Config {
                        bucket: bucket_name.to_string(),
                        history: 5,
                        ..Default::default()
                    })
                    .await
                    .map_err(|e| BusError::StreamNotFound(e.to_string()))?;
                }
            }
        }

        Ok(())
    }

    // === Type Definitions ===

    /// Store a validated type definition.
    pub async fn put_type(&self, type_def: TypeDef) -> Result<(), BusError> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let store = js
            .get_key_value(BUCKET_TYPES)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        let value = serde_json::to_vec(&type_def)
            .map_err(|e| BusError::SerializationFailed(e.to_string()))?;

        store
            .put(&type_def.name, value.into())
            .await
            .map_err(|e| BusError::PublishFailed(e.to_string()))?;

        // Update cache
        let mut cache = self.types_cache.write().await;
        cache.insert(type_def.name.clone(), type_def);

        Ok(())
    }

    /// Get a type definition by name.
    pub async fn get_type(&self, name: &str) -> Result<Option<TypeDef>, BusError> {
        // Check cache first
        {
            let cache = self.types_cache.read().await;
            if let Some(t) = cache.get(name) {
                return Ok(Some(t.clone()));
            }
        }

        // Fetch from NATS KV
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let store = js
            .get_key_value(BUCKET_TYPES)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        match store.get(name).await {
            Ok(Some(entry)) => {
                let type_def: TypeDef = serde_json::from_slice(&entry)
                    .map_err(|e| BusError::DeserializationFailed(e.to_string()))?;

                // Update cache
                let mut cache = self.types_cache.write().await;
                cache.insert(name.to_string(), type_def.clone());

                Ok(Some(type_def))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(BusError::SubscribeFailed(e.to_string())),
        }
    }

    /// Get all validated types.
    pub async fn get_all_types(&self) -> Result<Vec<TypeDef>, BusError> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let store = js
            .get_key_value(BUCKET_TYPES)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        let mut types = Vec::new();
        let mut keys = store
            .keys()
            .await
            .map_err(|e| BusError::SubscribeFailed(e.to_string()))?;

        while let Some(Ok(key)) = keys.next().await {
            if let Ok(Some(entry)) = store.get(&key).await {
                if let Ok(type_def) = serde_json::from_slice::<TypeDef>(&entry) {
                    types.push(type_def);
                }
            }
        }

        Ok(types)
    }

    // === Function Signatures ===

    /// Store a validated function signature.
    pub async fn put_function(&self, func: FunctionSig) -> Result<(), BusError> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let store = js
            .get_key_value(BUCKET_FUNCTIONS)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        // Key format: TypeName::method_name or ::function_name for free functions
        let key = match &func.impl_for {
            Some(type_name) => format!("{}::{}", type_name, func.name),
            None => format!("::{}", func.name),
        };

        let value =
            serde_json::to_vec(&func).map_err(|e| BusError::SerializationFailed(e.to_string()))?;

        store
            .put(&key, value.into())
            .await
            .map_err(|e| BusError::PublishFailed(e.to_string()))?;

        // Update cache
        let mut cache = self.functions_cache.write().await;
        cache.insert(key, func);

        Ok(())
    }

    /// Get a function signature.
    pub async fn get_function(
        &self,
        type_name: Option<&str>,
        func_name: &str,
    ) -> Result<Option<FunctionSig>, BusError> {
        let key = match type_name {
            Some(t) => format!("{}::{}", t, func_name),
            None => format!("::{}", func_name),
        };

        // Check cache first
        {
            let cache = self.functions_cache.read().await;
            if let Some(f) = cache.get(&key) {
                return Ok(Some(f.clone()));
            }
        }

        // Fetch from NATS KV
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let store = js
            .get_key_value(BUCKET_FUNCTIONS)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        match store.get(&key).await {
            Ok(Some(entry)) => {
                let func: FunctionSig = serde_json::from_slice(&entry)
                    .map_err(|e| BusError::DeserializationFailed(e.to_string()))?;

                let mut cache = self.functions_cache.write().await;
                cache.insert(key, func.clone());

                Ok(Some(func))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(BusError::SubscribeFailed(e.to_string())),
        }
    }

    /// Get all functions for a type.
    pub async fn get_methods(&self, type_name: &str) -> Result<Vec<FunctionSig>, BusError> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let store = js
            .get_key_value(BUCKET_FUNCTIONS)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        let prefix = format!("{}::", type_name);
        let mut functions = Vec::new();

        let mut keys = store
            .keys()
            .await
            .map_err(|e| BusError::SubscribeFailed(e.to_string()))?;

        while let Some(Ok(key)) = keys.next().await {
            if key.starts_with(&prefix) {
                if let Ok(Some(entry)) = store.get(&key).await {
                    if let Ok(func) = serde_json::from_slice::<FunctionSig>(&entry) {
                        functions.push(func);
                    }
                }
            }
        }

        Ok(functions)
    }

    // === Imports ===

    /// Store validated imports.
    pub async fn put_imports(&self, imports: Vec<String>) -> Result<(), BusError> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let store = js
            .get_key_value(BUCKET_IMPORTS)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        let value = serde_json::to_vec(&imports)
            .map_err(|e| BusError::SerializationFailed(e.to_string()))?;

        store
            .put("validated_imports", value.into())
            .await
            .map_err(|e| BusError::PublishFailed(e.to_string()))?;

        let mut cache = self.imports_cache.write().await;
        *cache = imports;

        Ok(())
    }

    /// Get validated imports.
    pub async fn get_imports(&self) -> Result<Vec<String>, BusError> {
        {
            let cache = self.imports_cache.read().await;
            if !cache.is_empty() {
                return Ok(cache.clone());
            }
        }

        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let store = js
            .get_key_value(BUCKET_IMPORTS)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        match store.get("validated_imports").await {
            Ok(Some(entry)) => {
                let imports: Vec<String> = serde_json::from_slice(&entry)
                    .map_err(|e| BusError::DeserializationFailed(e.to_string()))?;

                let mut cache = self.imports_cache.write().await;
                *cache = imports.clone();

                Ok(imports)
            }
            Ok(None) => Ok(vec![]),
            Err(e) => Err(BusError::SubscribeFailed(e.to_string())),
        }
    }

    // === Reference Patterns ===

    /// Store a reference pattern for injection into prompts.
    pub async fn put_pattern(&self, name: &str, code: &str) -> Result<(), BusError> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let store = js
            .get_key_value(BUCKET_PATTERNS)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        store
            .put(name, code.to_string().into_bytes().into())
            .await
            .map_err(|e| BusError::PublishFailed(e.to_string()))?;

        info!("Stored pattern: {}", name);
        Ok(())
    }

    /// Get a reference pattern by name.
    pub async fn get_pattern(&self, name: &str) -> Result<Option<String>, BusError> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let store = js
            .get_key_value(BUCKET_PATTERNS)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        match store.get(name).await {
            Ok(Some(entry)) => {
                let pattern = String::from_utf8_lossy(&entry).to_string();
                Ok(Some(pattern))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(BusError::SubscribeFailed(e.to_string())),
        }
    }

    /// Get all stored patterns.
    pub async fn get_all_patterns(&self) -> Result<HashMap<String, String>, BusError> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let store = js
            .get_key_value(BUCKET_PATTERNS)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        let mut patterns = HashMap::new();
        let mut keys = store
            .keys()
            .await
            .map_err(|e| BusError::SubscribeFailed(e.to_string()))?;

        while let Some(Ok(key)) = keys.next().await {
            if let Ok(Some(entry)) = store.get(&key).await {
                let code = String::from_utf8_lossy(&entry).to_string();
                patterns.insert(key, code);
            }
        }

        Ok(patterns)
    }

    // === Context Generation ===

    /// Generate a context string with all validated definitions.
    /// This can be prepended to LLM prompts to prevent type mismatches.
    pub async fn generate_context(&self) -> Result<String, BusError> {
        let mut context = String::new();

        // Add imports
        let imports = self.get_imports().await?;
        if !imports.is_empty() {
            context.push_str("// === VALIDATED IMPORTS ===\n");
            for import in imports {
                context.push_str(&import);
                context.push('\n');
            }
            context.push('\n');
        }

        // Add type definitions
        let types = self.get_all_types().await?;
        if !types.is_empty() {
            context.push_str("// === VALIDATED TYPE DEFINITIONS ===\n");
            context.push_str("// You MUST use these exact definitions. Do NOT modify them.\n\n");
            for t in types {
                context.push_str(&t.definition);
                context.push_str("\n\n");
            }
        }

        Ok(context)
    }

    /// Clear all stored state (for new pipeline run).
    pub async fn clear(&self) -> Result<(), BusError> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        for bucket_name in [
            BUCKET_TYPES,
            BUCKET_FUNCTIONS,
            BUCKET_FIELDS,
            BUCKET_IMPORTS,
        ] {
            if let Ok(store) = js.get_key_value(bucket_name).await {
                if let Ok(mut keys) = store.keys().await {
                    while let Some(Ok(key)) = keys.next().await {
                        let _ = store.delete(&key).await;
                    }
                }
            }
        }

        // Clear caches
        self.types_cache.write().await.clear();
        self.functions_cache.write().await.clear();
        self.imports_cache.write().await.clear();

        info!("StateStore cleared");
        Ok(())
    }
}

/// Parse type definitions from Rust code.
pub fn extract_types(code: &str) -> Vec<TypeDef> {
    let mut types = Vec::new();

    // Simple regex-based extraction
    // In production, use syn crate for proper parsing
    let struct_re = regex::Regex::new(
        r"(?m)^(?:#\[derive\([^\]]+\)\]\s*)?pub\s+struct\s+(\w+)(?:<[^>]+>)?\s*(?:\([^)]+\);|\{[^}]*\})"
    ).unwrap();

    let enum_re =
        regex::Regex::new(r"(?m)^(?:#\[derive\([^\]]+\)\]\s*)?pub\s+enum\s+(\w+)\s*\{[^}]*\}")
            .unwrap();

    for cap in struct_re.captures_iter(code) {
        if let Some(name) = cap.get(1) {
            types.push(TypeDef::new(name.as_str(), cap.get(0).unwrap().as_str()));
        }
    }

    for cap in enum_re.captures_iter(code) {
        if let Some(name) = cap.get(1) {
            types.push(TypeDef::new(name.as_str(), cap.get(0).unwrap().as_str()));
        }
    }

    types
}

/// Extract imports from Rust code.
pub fn extract_imports(code: &str) -> Vec<String> {
    let mut imports = Vec::new();
    let import_re = regex::Regex::new(r"(?m)^use\s+[^;]+;").unwrap();

    for m in import_re.find_iter(code) {
        imports.push(m.as_str().to_string());
    }

    imports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_types() {
        let code = r#"
#[derive(Debug, Clone)]
pub struct TaskId(u64);

#[derive(Debug)]
pub struct TaskDef {
    id: TaskId,
    name: String,
}

pub enum TaskError {
    Timeout,
    Failed,
}
"#;

        let types = extract_types(code);
        assert_eq!(types.len(), 3);
        assert!(types.iter().any(|t| t.name == "TaskId"));
        assert!(types.iter().any(|t| t.name == "TaskDef"));
        assert!(types.iter().any(|t| t.name == "TaskError"));
    }

    #[test]
    fn test_extract_imports() {
        let code = r#"
use std::collections::HashMap;
use tokio::sync::{Mutex, RwLock};
use std::time::Duration;
"#;

        let imports = extract_imports(code);
        assert_eq!(imports.len(), 3);
    }
}

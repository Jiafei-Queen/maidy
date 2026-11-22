//! 供应商 `/models` 发现结果的本地持久化缓存。
//!
//! 缓存与 `models.json` 解耦保存：只记录最近一次成功发现的模型 ID 列表和同步时间，
//! 不保存任何 URL、认证头或凭证，也不改动 `models.json` / `agent.toml` 的 revision。

use std::{collections::BTreeMap, path::Path};

use chrono::Utc;
use qq_maid_llm::provider::discovery::{DiscoveryState, ModelDiscovery};
use serde::{Deserialize, Serialize};

use super::{ConfigCenterError, managed_file};

const DISCOVERY_CACHE_SCHEMA_VERSION: u32 = 1;
const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedDiscoveredModel {
    pub id: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedDiscoveryEntry {
    pub state: String,
    pub models: Vec<CachedDiscoveredModel>,
    pub category: String,
    pub http_status: Option<u16>,
    pub elapsed_ms: u64,
    pub synced_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DiscoveryCacheDocument {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(default)]
    connections: BTreeMap<String, CachedDiscoveryEntry>,
}

fn default_schema_version() -> u32 {
    DISCOVERY_CACHE_SCHEMA_VERSION
}

fn load_document(path: &Path) -> Result<(String, DiscoveryCacheDocument), ConfigCenterError> {
    let bytes = managed_file::read_regular_file(path)?;
    let revision = bytes
        .as_deref()
        .map(managed_file::revision)
        .unwrap_or_else(|| "missing".to_owned());
    let Some(bytes) = bytes else {
        return Ok((revision, DiscoveryCacheDocument::default()));
    };
    let document = match serde_json::from_slice::<DiscoveryCacheDocument>(&bytes) {
        Ok(doc) if doc.schema_version == DISCOVERY_CACHE_SCHEMA_VERSION => doc,
        // 缓存损坏或版本过旧时安全回退为空，避免阻断模型元数据加载。
        _ => DiscoveryCacheDocument::default(),
    };
    Ok((revision, document))
}

/// 读取指定连接的最近一次成功发现缓存；未同步或缓存不存在时返回 `None`。
pub(super) fn load_cached_discovery(
    path: &Path,
    connection_id: &str,
) -> Result<Option<CachedDiscoveryEntry>, ConfigCenterError> {
    let (_, document) = load_document(path)?;
    let key = connection_id.trim().to_ascii_lowercase();
    Ok(document.connections.get(&key).cloned())
}

/// 仅在本次发现成功时写入持久化缓存，避免临时网络故障冲掉上次成功列表。
pub(super) fn save_cached_discovery(
    path: &Path,
    connection_id: &str,
    discovery: &ModelDiscovery,
) -> Result<Option<CachedDiscoveryEntry>, ConfigCenterError> {
    if discovery.state != DiscoveryState::Success {
        return Ok(None);
    }
    let models = discovery
        .models
        .as_ref()
        .map(|items| {
            items
                .iter()
                .map(|item| CachedDiscoveredModel {
                    id: item.id.clone(),
                    source: item.source.to_owned(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let entry = CachedDiscoveryEntry {
        state: "success".to_owned(),
        models,
        category: discovery.category.to_owned(),
        http_status: discovery.http_status,
        elapsed_ms: discovery.elapsed_ms,
        synced_at: Utc::now().to_rfc3339(),
    };
    let (expected_revision, mut document) = load_document(path)?;
    document.schema_version = DISCOVERY_CACHE_SCHEMA_VERSION;
    let key = connection_id.trim().to_ascii_lowercase();
    document.connections.insert(key, entry.clone());
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| ConfigCenterError::invalid(format!("发现缓存序列化失败：{error}")))?;
    if bytes.len() > MAX_CACHE_BYTES {
        return Err(ConfigCenterError::invalid("发现缓存超出 2 MiB 上限"));
    }
    managed_file::atomic_write_if_revision(
        path,
        &bytes,
        &expected_revision,
        "models discovery cache",
    )?;
    Ok(Some(entry))
}

/// 删除连接时清理对应的发现缓存；缓存缺失时不报错。
pub(super) fn remove_cached_discovery(
    path: &Path,
    connection_id: &str,
) -> Result<(), ConfigCenterError> {
    let (expected_revision, mut document) = load_document(path)?;
    if expected_revision == "missing" {
        return Ok(());
    }
    let key = connection_id.trim().to_ascii_lowercase();
    if document.connections.remove(&key).is_none() {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| ConfigCenterError::invalid(format!("发现缓存序列化失败：{error}")))?;
    managed_file::atomic_write_if_revision(
        path,
        &bytes,
        &expected_revision,
        "models discovery cache",
    )
}

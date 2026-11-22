//! 通用 Connection 管理适配；不迁移内置配置或已有环境变量引用。

use super::*;
use qq_maid_llm::provider::discovery::DiscoveryAdapter;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use toml::Value;

const SLOT_PREFIX: &str = "QQ_MAID_CONNECTION_";
const SECRET_PREFIX: &str = "connection.";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConnectionCredentialStatus {
    pub configured: bool,
    pub editable: bool,
    pub revision: String,
    pub pending_restart: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProviderManagementSnapshot {
    pub presets: Vec<ProviderPreset>,
    pub adapters: [&'static str; 2],
    /// 为 true 时前端只渲染 `agent.toml [providers.*]` 统一列表，不再硬编码不可删内置卡片。
    pub managed_providers: bool,
    pub credentials: BTreeMap<String, ConnectionCredentialStatus>,
    /// 各供应商当前可供路由下拉选择的模型 ID（优先取精选白名单，未配置时取已发现与本地启用模型）。
    pub available_models: BTreeMap<String, Vec<String>>,
}

fn controlled_slot(value: &str) -> bool {
    value.strip_prefix(SLOT_PREFIX).is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

impl ConfigCenter {
    pub async fn discover_connection_models(
        &self,
        id: &str,
        expected_revision: &str,
    ) -> Result<qq_maid_llm::provider::discovery::ModelDiscovery, ConfigCenterError> {
        let connection = self.resolve_connection_for_read(id, expected_revision)?;
        let discovery = qq_maid_llm::provider::discovery::discover_models(
            connection.discovery_adapter,
            &connection.base_url,
            &connection.auth,
            &connection.api_key,
            std::time::Duration::from_secs(connection.timeout_seconds),
        )
        .await
        .map_err(|error| ConfigCenterError::invalid(error.message))?;
        if discovery.state == qq_maid_llm::provider::discovery::DiscoveryState::Success {
            let _guard = self
                .mutation_lock
                .lock()
                .map_err(|_| ConfigCenterError::io("configuration mutation lock is poisoned"))?;
            // 仅持久化最近一次成功发现的模型 ID 列表，不修改 models.json / agent.toml revision。
            if let Err(error) = super::discovery_cache::save_cached_discovery(
                &self.discovery_cache_path(),
                id,
                &discovery,
            ) {
                tracing::warn!(
                    connection_id = id,
                    error = %error,
                    "供应商模型发现缓存写入失败，仍返回本次实时发现结果"
                );
            }
        }
        Ok(discovery)
    }

    pub async fn test_provider_connection(
        &self,
        id: &str,
        expected_revision: &str,
        model: &str,
    ) -> Result<qq_maid_llm::provider::openai::diagnostics::ConnectionDiagnostic, ConfigCenterError>
    {
        let connection = self.resolve_connection_for_read(id, expected_revision)?;
        qq_maid_llm::provider::openai::diagnostics::test_connection(
            &connection.base_url,
            // 诊断与运行时共用旧模型协议规则；自定义 Connection 仍尊重显式 kind。
            connection.responses
                && !(id == "deepseek"
                    && qq_maid_llm::provider::deepseek::is_legacy_chat_model(model.trim())),
            &connection.auth,
            &connection.api_key,
            model,
        )
        .await
        .map_err(|error| ConfigCenterError::invalid(error.message))
    }

    // 锁内校验 revision 并提取保存配置与凭证；网络等待不占用配置写锁。
    fn resolve_connection_for_read(
        &self,
        id: &str,
        expected_revision: &str,
    ) -> Result<ResolvedConnection, ConfigCenterError> {
        let _guard = self
            .mutation_lock
            .lock()
            .map_err(|_| ConfigCenterError::io("configuration mutation lock is poisoned"))?;
        let agent_snapshot = self
            .agent_file
            .as_ref()
            .map(AgentConfigFile::snapshot)
            .transpose()?;
        let saved_providers = agent_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.saved_value.as_ref())
            .and_then(|value| value.get("providers").and_then(Value::as_table).cloned())
            .unwrap_or_default();
        let managed_providers = agent_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.saved_value.as_ref())
            .and_then(|value| value.get("managed_providers").and_then(Value::as_bool))
            .unwrap_or(false);

        // 优先从统一的 agent.toml [providers.*] 读取；仅旧版未迁移配置才回退内置 env 连接。
        if let Some(provider) = saved_providers.get(id) {
            let snapshot = agent_snapshot.expect("saved provider implies agent snapshot");
            if snapshot.revision != expected_revision {
                return Err(ConfigCenterError::conflict(
                    "Connection 已修改，请刷新后测试",
                ));
            }
            let connection: super::AgentProviderUpdate = provider
                .clone()
                .try_into()
                .map_err(|_| ConfigCenterError::invalid("Connection 配置无效"))?;
            if !connection.enabled {
                return Err(ConfigCenterError::invalid("请先启用 Connection"));
            }
            let environment = self.current_resolved_environment()?;
            let key = environment
                .get(&connection.api_key_env)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| ConfigCenterError::invalid("Credential 尚未配置"))?;
            let responses =
                connection.kind == crate::config::agent::AgentProviderKind::OpenAiResponses;
            return Ok(ResolvedConnection {
                base_url: connection.base_url,
                responses,
                auth: qq_maid_llm::config::HttpAuthConfig {
                    header: connection.auth_header,
                    scheme: connection.auth_scheme,
                },
                api_key: key.to_owned(),
                timeout_seconds: discovery_timeout(
                    &environment,
                    connection.request_timeout_seconds,
                )?,
                discovery_adapter: if responses {
                    DiscoveryAdapter::OpenAiResponses
                } else {
                    DiscoveryAdapter::OpenAiCompatible
                },
            });
        }

        if !managed_providers && let Some(definition) = builtin_connection(id) {
            return self.resolve_builtin_connection_for_read(id, expected_revision, definition);
        }

        let snapshot =
            agent_snapshot.ok_or_else(|| ConfigCenterError::invalid("Agent 配置不可用"))?;
        if snapshot.revision != expected_revision {
            return Err(ConfigCenterError::conflict(
                "Connection 已修改，请刷新后测试",
            ));
        }
        Err(ConfigCenterError::invalid("Connection 不存在"))
    }

    fn resolve_builtin_connection_for_read(
        &self,
        id: &str,
        expected_revision: &str,
        (base_env, key_env, default_url): (&str, &str, &str),
    ) -> Result<ResolvedConnection, ConfigCenterError> {
        let saved = self.managed_file.load()?;
        if saved.revision != expected_revision {
            return Err(ConfigCenterError::conflict("内置连接已修改，请刷新后测试"));
        }
        let environment = self.current_resolved_environment()?;
        let enabled = environment
            .get(&format!("{}_ENABLED", id.to_ascii_uppercase()))
            .map(String::as_str)
            .unwrap_or("true");
        if matches!(
            enabled.trim().to_ascii_lowercase().as_str(),
            "false" | "0" | "off" | "no" | "disabled" | "none"
        ) {
            return Err(ConfigCenterError::invalid("请先启用 Connection"));
        }
        let key = environment
            .get(key_env)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ConfigCenterError::invalid("Credential 尚未配置"))?;
        let base = environment
            .get(base_env)
            .map(String::as_str)
            .unwrap_or(default_url);
        let base = base
            .split(',')
            .map(str::trim)
            .find(|value| !value.is_empty())
            .unwrap_or(default_url);
        // DeepSeek 默认使用 Responses；模型探针另按旧模型兼容规则选择 Chat。
        let responses = id == "deepseek"
            || id == "openai"
                && crate::config::parse_openai_api_mode(
                    environment
                        .get("OPENAI_API_MODE")
                        .map(String::as_str)
                        .unwrap_or("auto"),
                )
                .map_err(|error| ConfigCenterError::invalid(error.message))?
                    != crate::config::OpenAiApiMode::ChatOnly;
        Ok(ResolvedConnection {
            base_url: base.to_owned(),
            responses,
            auth: qq_maid_llm::config::HttpAuthConfig::default(),
            api_key: key.to_owned(),
            timeout_seconds: discovery_timeout(&environment, None)?,
            // 内置连接除 Responses 外均使用 Chat Completions adapter；
            // /models 是否可用由端点响应判定，不按 Provider 品牌提前拒绝。
            discovery_adapter: if responses {
                DiscoveryAdapter::OpenAiResponses
            } else {
                DiscoveryAdapter::OpenAiCompatible
            },
        })
    }

    pub fn provider_snapshot(&self) -> Result<ProviderManagementSnapshot, ConfigCenterError> {
        let mut credentials = BTreeMap::new();
        let secrets = self.secret_store.envelope_revisions()?;
        let environment = self.current_resolved_environment()?;
        let saved_providers = self.saved_providers()?;
        for (id, provider) in &saved_providers {
            let slot = provider
                .get("api_key_env")
                .and_then(Value::as_str)
                .unwrap_or("");
            let key = self.connection_secret_key(slot);
            let revision = key
                .as_ref()
                .and_then(|key| secrets.get(key))
                .cloned()
                .unwrap_or_else(|| SECRET_MISSING_REVISION.to_owned());
            let pending_restart = key
                .as_ref()
                .is_some_and(|key| secrets.get(key) != self.running_secret_revisions.get(key));
            credentials.insert(
                id.clone(),
                ConnectionCredentialStatus {
                    configured: environment
                        .get(slot)
                        .is_some_and(|value| !value.trim().is_empty()),
                    editable: key.is_some(),
                    revision,
                    pending_restart,
                },
            );
        }
        let managed_providers = self
            .agent_file
            .as_ref()
            .map(AgentConfigFile::snapshot)
            .transpose()?
            .and_then(|snapshot| snapshot.saved_value)
            .and_then(|value| value.get("managed_providers").and_then(Value::as_bool))
            .unwrap_or(false);
        let available_models = self.collect_available_models(&saved_providers, managed_providers);
        Ok(ProviderManagementSnapshot {
            presets: provider_presets(),
            adapters: ["openai_compatible", "openai_responses"],
            managed_providers,
            credentials,
            available_models,
        })
    }

    /// 汇总各供应商当前可供路由下拉选择的模型列表（不含凭据，不触发上游网络请求）。
    fn collect_available_models(
        &self,
        saved_providers: &toml::map::Map<String, Value>,
        managed_providers: bool,
    ) -> BTreeMap<String, Vec<String>> {
        let local = self.load_models().ok().map(|(_, local)| local);
        let discoveries =
            super::discovery_cache::load_all_cached_discoveries(&self.discovery_cache_path())
                .unwrap_or_default();

        let mut provider_ids: Vec<String> = saved_providers.keys().cloned().collect();
        if !managed_providers {
            let existing_lower: HashSet<String> = provider_ids
                .iter()
                .map(|id| id.to_ascii_lowercase())
                .collect();
            for builtin in ["openai", "deepseek", "bigmodel", "gemini"] {
                if !existing_lower.contains(builtin) {
                    provider_ids.push(builtin.to_owned());
                }
            }
        }

        let mut result = BTreeMap::new();
        for raw_id in provider_ids {
            let canonical = raw_id.to_ascii_lowercase();
            let (identity, _) = crate::config::model_management::catalog_identity(&canonical);
            let disabled_ids: HashSet<&str> = local
                .as_ref()
                .map(|doc| {
                    doc.models
                        .iter()
                        .filter(|m| m.provider.as_str() == identity && m.enabled == Some(false))
                        .map(|m| m.id.as_str())
                        .collect()
                })
                .unwrap_or_default();

            let mut seen = HashSet::new();
            let mut models = Vec::new();
            let mut push_if_allowed = |id: &str| {
                let trimmed = id.trim();
                if !trimmed.is_empty()
                    && !disabled_ids.contains(trimmed)
                    && seen.insert(trimmed.to_owned())
                {
                    models.push(trimmed.to_owned());
                }
            };

            let curated = local.as_ref().and_then(|doc| {
                qq_maid_llm::model_catalog::CatalogProviderId::parse(&canonical)
                    .ok()
                    .and_then(|key| doc.selections.get(&key))
            });
            if let Some(selected) = curated {
                for id in selected {
                    push_if_allowed(id);
                }
            } else {
                if let Some(cached) = discoveries.get(&canonical) {
                    for item in &cached.models {
                        push_if_allowed(&item.id);
                    }
                }
                if let Some(doc) = &local {
                    for entry in &doc.models {
                        if entry.provider.as_str() == identity && entry.enabled != Some(false) {
                            push_if_allowed(&entry.id);
                        }
                    }
                }
            }
            if !models.is_empty() {
                result.insert(raw_id, models);
            }
        }
        result
    }

    fn saved_providers(&self) -> Result<toml::map::Map<String, Value>, ConfigCenterError> {
        Ok(self
            .agent_file
            .as_ref()
            .map(AgentConfigFile::snapshot)
            .transpose()?
            .and_then(|snapshot| snapshot.saved_value)
            .and_then(|value| value.get("providers").and_then(Value::as_table).cloned())
            .unwrap_or_default())
    }

    fn connection_secret_key(&self, slot: &str) -> Option<String> {
        if controlled_slot(slot) {
            return Some(format!("{SECRET_PREFIX}{slot}"));
        }
        self.registry
            .fields()
            .iter()
            .find(|field| {
                field.env_name == slot
                    && field.sensitivity == ManagedConfigSensitivity::Secret
                    && field.web_editable
            })
            .map(|field| field.key.to_owned())
    }

    pub(super) fn prepare_provider_changes(
        &self,
        changes: &[AgentConfigChange],
    ) -> Result<Vec<AgentConfigChange>, ConfigCenterError> {
        let saved = self.saved_providers()?;
        // agent.toml 历史手工配置允许非 canonical key；管理 API 只按精确匹配编辑，
        // canonical ID 用于拒绝 MyProxy / myproxy 这类大小写别名重复。
        let mut saved_by_canonical_id = HashMap::new();
        for saved_id in saved.keys() {
            let canonical_id = provider_canonical_id(saved_id)?;
            if saved_by_canonical_id
                .insert(canonical_id.clone(), saved_id.clone())
                .is_some()
            {
                return Err(ConfigCenterError::invalid(format!(
                    "duplicate provider `{canonical_id}`"
                )));
            }
        }
        let mut seen = HashSet::new();
        let mut changes = changes.to_vec();
        for change in &mut changes {
            match change {
                AgentConfigChange::SetProvider { id, provider } => {
                    let parsed = qq_maid_llm::provider::types::ModelProvider::parse_prefix(id)
                        .map_err(|_| ConfigCenterError::invalid("Connection ID 格式无效"))?;
                    let canonical_id = parsed.as_str().to_owned();
                    if !seen.insert(canonical_id.clone()) {
                        return Err(ConfigCenterError::invalid(
                            "同一请求不能重复修改 Connection",
                        ));
                    }
                    if id.len() > 64 || id.trim() != id {
                        return Err(ConfigCenterError::invalid(
                            "Connection ID 不能包含首尾空白，且不超过 64 字符",
                        ));
                    }
                    if let Some(existing) = saved.get(id) {
                        let slot = existing
                            .get("api_key_env")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if !provider.api_key_env.is_empty() && provider.api_key_env != slot {
                            return Err(ConfigCenterError::invalid(
                                "不能修改已有 Connection 的 Credential 引用",
                            ));
                        }
                        provider.api_key_env = slot.to_owned();
                    } else if saved_by_canonical_id.contains_key(&canonical_id) {
                        return Err(ConfigCenterError::invalid(
                            "Connection 已存在，必须使用保存配置中的原始 ID，且不能改名",
                        ));
                    } else if parsed.as_str() != id {
                        return Err(ConfigCenterError::invalid(
                            "Connection ID 必须为小写，且不超过 64 字符",
                        ));
                    } else {
                        // 旧版 OpenCode 共用 OPENCODE_API_KEY 继续兼容；新建其余连接均由服务端生成隔离 Slot。
                        let legacy_opencode = ["opencode_zen", "opencode_zen_chat", "opencode_go"]
                            .contains(&id.as_str())
                            && provider.api_key_env == "OPENCODE_API_KEY";
                        if !provider.api_key_env.is_empty() && !legacy_opencode {
                            return Err(ConfigCenterError::invalid(
                                "新 Connection 的 Credential Slot 必须由服务端生成",
                            ));
                        }
                        if !legacy_opencode {
                            provider.api_key_env =
                                format!("{SLOT_PREFIX}{}", uuid::Uuid::new_v4().simple());
                        }
                    }
                }
                // 专属密文归档留在加密存储中。重建同名 Connection 分配新的随机 Slot，
                // 永不重新绑定旧密文；避免跨 TOML / SQLite 非原子删除造成凭证丢失。
                AgentConfigChange::RemoveProvider { id, .. } => {
                    let canonical_id = provider_canonical_id(id)?;
                    let exact_saved = saved.contains_key(id);
                    let canonical_saved = saved_by_canonical_id.contains_key(&canonical_id);
                    if !seen.insert(canonical_id) {
                        return Err(ConfigCenterError::invalid(
                            "同一请求不能重复修改 Connection",
                        ));
                    }
                    if !exact_saved && canonical_saved {
                        return Err(ConfigCenterError::invalid(
                            "Connection 已存在，必须使用保存配置中的原始 ID，且不能改名",
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(changes)
    }

    pub fn update_connection_credential(
        &self,
        id: &str,
        expected_agent_revision: &str,
        expected_revision: &str,
        value: Option<&str>,
    ) -> Result<(), ConfigCenterError> {
        let _guard = self
            .mutation_lock
            .lock()
            .map_err(|_| ConfigCenterError::io("configuration mutation lock is poisoned"))?;
        let agent = self
            .agent_file
            .as_ref()
            .ok_or_else(|| ConfigCenterError::invalid("Agent 配置不可用"))?
            .snapshot()?;
        if agent.revision != expected_agent_revision {
            return Err(ConfigCenterError::conflict("Connection 已修改，请刷新页面"));
        }
        let providers = self.saved_providers()?;
        let slot = providers
            .get(id)
            .and_then(|provider| provider.get("api_key_env"))
            .and_then(Value::as_str)
            .ok_or_else(|| ConfigCenterError::invalid("Connection 不存在"))?;
        let key = self
            .connection_secret_key(slot)
            .ok_or_else(|| ConfigCenterError::invalid("历史环境变量凭证仅可在部署环境修改"))?;
        let change = match value {
            Some(value) => {
                validate_secret_replacement(&key, value)?;
                SecretConfigChange::Replace {
                    key,
                    value: value.to_owned(),
                    expected_revision: expected_revision.to_owned(),
                }
            }
            None => SecretConfigChange::Clear {
                key,
                expected_revision: expected_revision.to_owned(),
            },
        };
        let managed = self.managed_file.load()?;
        self.secret_store.mutate(&[change], |secrets| {
            let environment = self.resolve_environment_from(
                &managed.values,
                secrets,
                &self.external_environment,
            )?;
            self.validate_candidate_for_write(&environment, None)
        })?;
        Ok(())
    }

    pub(super) fn resolve_connection_credentials(
        &self,
        secrets: &HashMap<String, Vec<u8>>,
        resolved: &mut HashMap<String, String>,
    ) -> Result<(), ConfigCenterError> {
        for (key, value) in secrets {
            if let Some(slot) = key
                .strip_prefix(SECRET_PREFIX)
                .filter(|slot| controlled_slot(slot))
            {
                let value = String::from_utf8(value.clone())
                    .map_err(|_| ConfigCenterError::secret("Connection Credential 编码无效"))?;
                resolved.insert(slot.to_owned(), value);
            }
        }
        Ok(())
    }
}

fn provider_canonical_id(id: &str) -> Result<String, ConfigCenterError> {
    let parsed = qq_maid_llm::provider::types::ModelProvider::parse_prefix(id)
        .map_err(|_| ConfigCenterError::invalid("Connection ID 格式无效"))?;
    Ok(parsed.as_str().to_owned())
}

/// 内置 Connection 继续使用原 runtime / Secret 字段；诊断不触发任何配置迁移。
fn builtin_connection(id: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match id {
        "openai" => Some((
            "OPENAI_BASE_URLS",
            "OPENAI_API_KEY",
            "https://api.openai.com/v1",
        )),
        "deepseek" => Some((
            "DEEPSEEK_BASE_URL",
            "DEEPSEEK_API_KEY",
            "https://api.deepseek.com",
        )),
        "bigmodel" => Some((
            "BIGMODEL_BASE_URL",
            "BIGMODEL_API_KEY",
            "https://open.bigmodel.cn/api/paas/v4",
        )),
        "gemini" => Some((
            "GEMINI_BASE_URL",
            "GEMINI_API_KEY",
            "https://generativelanguage.googleapis.com/v1beta/openai",
        )),
        _ => None,
    }
}

// 含明文凭证的临时结构不可 Debug / Serialize，也不得持久化。
struct ResolvedConnection {
    base_url: String,
    responses: bool,
    auth: qq_maid_llm::config::HttpAuthConfig,
    api_key: String,
    timeout_seconds: u64,
    discovery_adapter: DiscoveryAdapter,
}

fn discovery_timeout(
    environment: &HashMap<String, String>,
    custom: Option<u64>,
) -> Result<u64, ConfigCenterError> {
    let seconds = match custom {
        Some(seconds) => seconds,
        None => environment
            .get("LLM_REQUEST_TIMEOUT_SECONDS")
            .map(|value| value.trim().parse::<u64>())
            .transpose()
            .map_err(|_| ConfigCenterError::invalid("连接超时配置无效"))?
            .unwrap_or(180),
    };
    if seconds == 0 {
        return Err(ConfigCenterError::invalid("连接超时必须大于零"));
    }
    Ok(seconds)
}

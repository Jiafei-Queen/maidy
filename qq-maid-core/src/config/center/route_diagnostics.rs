//! 配置中心路由健康诊断：检查文本路由、搜索路由与 STT 路由中各候选的可用性。
//!
//! 停用供应商允许直接保存（运行时跳过已停用候选），本模块把不可用原因结构化暴露给
//! Web 控制台，便于在停用前提示影响并在模型路由页标红/黄告警。

use std::collections::{HashMap, HashSet};

use qq_maid_llm::{
    model_catalog::{CatalogProviderId, EffectiveModelCatalog},
    provider::types::{ModelId, ModelProvider},
    web_search::WebSearchBackend,
};
use serde::Serialize;

use crate::config::{
    AgentRuntimeConfig, is_platform_stt_model, model_management::catalog_identity,
};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteDiagnosticKind {
    ModelRoute,
    SearchRoute,
    SttRoute,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateIssueCode {
    ProviderDisabled,
    ProviderMissing,
    MissingCredential,
    ModelDisabled,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CandidateDiagnostic {
    pub index: usize,
    pub candidate: String,
    pub provider: String,
    pub model: String,
    pub available: bool,
    pub issues: Vec<CandidateIssueCode>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RouteDiagnostic {
    pub kind: RouteDiagnosticKind,
    pub route_name: String,
    /// 当前业务路径是否启用该路线（例如非 provider_native 时搜索路线为 false）。
    pub active: bool,
    pub has_available_candidate: bool,
    pub candidates: Vec<CandidateDiagnostic>,
}

struct ProviderAvailability {
    enabled: bool,
    credential_configured: bool,
}

/// 汇总当前已保存配置下全部路由的候选可用性；不返回任何密钥原文。
pub(super) fn collect_route_diagnostics(
    agent: &AgentRuntimeConfig,
    environment: &HashMap<String, String>,
    catalog: &EffectiveModelCatalog,
) -> Vec<RouteDiagnostic> {
    let availability = build_provider_availability(agent, environment);
    let mut diagnostics = Vec::new();

    // 1. 文本模型候选路线
    let mut model_routes = agent.configured_model_routes();
    model_routes.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (route_name, route) in model_routes {
        let short_name = route_name
            .strip_prefix("agent.model_routes.")
            .unwrap_or(&route_name)
            .to_owned();
        let candidates = route
            .candidates()
            .iter()
            .enumerate()
            .map(|(index, model)| {
                diagnose_candidate(
                    index,
                    model,
                    &ModelProvider::OpenAi,
                    &availability,
                    Some(catalog),
                )
            })
            .collect::<Vec<_>>();
        let has_available_candidate = candidates.iter().any(|candidate| candidate.available);
        diagnostics.push(RouteDiagnostic {
            kind: RouteDiagnosticKind::ModelRoute,
            route_name: short_name,
            active: true,
            has_available_candidate,
            candidates,
        });
    }

    // 2. 联网搜索模型路线（仅 provider_native 后端实际消费）
    if let Some(document) = agent.document() {
        let search_active = agent.web_search().default_backend == WebSearchBackend::ProviderNative;
        let mut search_routes = document.tools.web_search.routes.iter().collect::<Vec<_>>();
        search_routes.sort_by(|(left, _), (right, _)| left.cmp(right));
        for (route_name, route) in search_routes {
            let candidates = match ModelId::parse_config(&route.model, route_name) {
                Ok(model) => vec![diagnose_candidate(
                    0,
                    &model,
                    &ModelProvider::OpenAi,
                    &availability,
                    Some(catalog),
                )],
                Err(_) => Vec::new(),
            };
            let has_available_candidate = candidates.iter().any(|candidate| candidate.available);
            diagnostics.push(RouteDiagnostic {
                kind: RouteDiagnosticKind::SearchRoute,
                route_name: route_name.clone(),
                active: search_active,
                has_available_candidate,
                candidates,
            });
        }
    }

    // 3. 语音转写（STT）路线
    let stt = agent.stt();
    for (route_name, models) in [
        ("default", &stt.routes.default),
        ("private", &stt.routes.private),
        ("group", &stt.routes.group),
    ] {
        if models.is_empty() {
            continue;
        }
        let candidates = models
            .iter()
            .enumerate()
            .map(|(index, model)| {
                if is_platform_stt_model(model) {
                    return CandidateDiagnostic {
                        index,
                        candidate: model.to_request_model(),
                        provider: "platform".to_owned(),
                        model: model.name.clone(),
                        available: true,
                        issues: Vec::new(),
                        message: None,
                    };
                }
                // STT 不受文本模型 catalog enabled 限制，只检查连接启停与凭据。
                diagnose_candidate(index, model, &ModelProvider::OpenAi, &availability, None)
            })
            .collect::<Vec<_>>();
        let has_available_candidate = candidates.iter().any(|candidate| candidate.available);
        diagnostics.push(RouteDiagnostic {
            kind: RouteDiagnosticKind::SttRoute,
            route_name: route_name.to_owned(),
            active: stt.enabled,
            has_available_candidate,
            candidates,
        });
    }

    diagnostics
}

fn build_provider_availability(
    agent: &AgentRuntimeConfig,
    environment: &HashMap<String, String>,
) -> HashMap<String, ProviderAvailability> {
    let mut map = HashMap::new();
    for (id, prefix) in [
        ("openai", "OPENAI"),
        ("deepseek", "DEEPSEEK"),
        ("bigmodel", "BIGMODEL"),
        ("gemini", "GEMINI"),
    ] {
        let enabled = !environment
            .get(&format!("{prefix}_ENABLED"))
            .is_some_and(|raw| is_false_env(raw));
        let credential_configured = environment
            .get(&format!("{prefix}_API_KEY"))
            .is_some_and(|raw| !raw.trim().is_empty());
        map.insert(
            id.to_owned(),
            ProviderAvailability {
                enabled,
                credential_configured,
            },
        );
    }
    let mut seen_custom = HashSet::new();
    for provider in agent.provider_configs() {
        let id = provider.id.as_str().to_owned();
        if !seen_custom.insert(id.clone()) {
            continue;
        }
        let credential_configured = environment
            .get(&provider.api_key_env)
            .is_some_and(|raw| !raw.trim().is_empty());
        map.insert(
            id,
            ProviderAvailability {
                enabled: provider.enabled,
                credential_configured,
            },
        );
    }
    map
}

fn diagnose_candidate(
    index: usize,
    model: &ModelId,
    default_provider: &ModelProvider,
    availability: &HashMap<String, ProviderAvailability>,
    catalog: Option<&EffectiveModelCatalog>,
) -> CandidateDiagnostic {
    let provider_id = model
        .provider
        .as_ref()
        .unwrap_or(default_provider)
        .as_str()
        .to_owned();
    let mut issues = Vec::new();
    let mut reasons = Vec::new();

    match availability.get(&provider_id) {
        None => {
            issues.push(CandidateIssueCode::ProviderMissing);
            reasons.push(format!("供应商 {provider_id} 未配置"));
        }
        Some(state) => {
            if !state.enabled {
                issues.push(CandidateIssueCode::ProviderDisabled);
                reasons.push(format!("供应商 {provider_id} 已停用，运行时将跳过"));
            }
            if !state.credential_configured {
                issues.push(CandidateIssueCode::MissingCredential);
                reasons.push(format!("供应商 {provider_id} 未配置 API Key"));
            }
        }
    }

    if let Some(catalog) = catalog {
        let (identity, _) = catalog_identity(&provider_id);
        if let Ok(catalog_provider) = CatalogProviderId::parse(identity)
            && catalog.is_model_enabled(Some(&catalog_provider), &model.name) == Some(false)
        {
            issues.push(CandidateIssueCode::ModelDisabled);
            reasons.push(format!("模型 {provider_id}:{} 已禁用", model.name));
        }
    }

    let available = issues.is_empty();
    let message = (!reasons.is_empty()).then(|| reasons.join("；"));
    CandidateDiagnostic {
        index,
        candidate: model.to_request_model(),
        provider: provider_id,
        model: model.name.clone(),
        available,
        issues,
        message,
    }
}

fn is_false_env(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "false" | "0" | "off" | "no" | "disabled" | "none"
    )
}

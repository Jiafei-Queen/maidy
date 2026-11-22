use super::*;

fn base_config(extra: &str) -> String {
    format!(
        r#"
version = 1

[model_routes.main]
candidates = ["openai:gpt-main"]

[profiles.balanced]
main_route = "main"

[scenes.private]
enabled = true
profile = "balanced"

[scenes.group]
enabled = true
profile = "balanced"

[tools.web_search.routes.search]
model = "openai:gpt-search"

{extra}
"#
    )
}

fn parse(extra: &str) -> Result<AgentRuntimeConfig, LlmError> {
    AgentRuntimeConfig::from_toml(
        &base_config(extra),
        AgentConfigSource::File("config/agent.toml".to_owned()),
    )
}

#[test]
fn missing_stt_section_defaults_to_disabled_and_platform_auto() {
    let config = parse("").unwrap();
    let stt = config.stt();
    assert!(!stt.enabled);
    assert_eq!(stt.platform_asr, SttPlatformAsrMode::Auto);
    assert!(stt.routes.is_empty());
    assert_eq!(stt.timeout_seconds, 30);
    assert_eq!(stt.platform_asr_attempts, 2);
}

#[test]
fn scene_chain_falls_back_to_default() {
    let config = parse(
        r#"
[stt]
enabled = true
[stt.routes]
default = ["openai:whisper-1"]
private = []
group = ["bigmodel:glm-asr-2512"]
"#,
    )
    .unwrap();
    let stt = config.stt();
    assert_eq!(
        stt.routes
            .for_scene(ChatScene::Private)
            .iter()
            .map(|model| model.to_request_model())
            .collect::<Vec<_>>(),
        ["openai:whisper-1"]
    );
    assert_eq!(
        stt.routes
            .for_scene(ChatScene::Group)
            .iter()
            .map(|model| model.to_request_model())
            .collect::<Vec<_>>(),
        ["bigmodel:glm-asr-2512"]
    );
}

#[test]
fn enabled_stt_requires_at_least_one_route() {
    let error = parse("[stt]\nenabled = true\n").unwrap_err();
    assert!(error.message.contains("no transcription model route"));
}

#[test]
fn disabled_stt_allows_empty_routes() {
    let config = parse("[stt]\nenabled = false\n[stt.routes]\ndefault = []\n").unwrap();
    assert!(!config.stt().enabled);
}

#[test]
fn disabled_stt_tolerates_stale_provider_references() {
    let config = parse(
        r#"
[stt]
enabled = false
[stt.routes]
default = ["removed_provider:whisper-1"]
"#,
    )
    .unwrap();
    assert_eq!(config.stt().routes.default.len(), 1);
    assert_eq!(
        config.stt().routes.default[0].to_request_model(),
        "removed_provider:whisper-1"
    );
}

#[test]
fn platform_candidate_is_accepted_without_real_provider() {
    let config = parse(
        r#"
[stt]
enabled = true
[stt.routes]
default = ["platform:qq", "openai:whisper-1"]
"#,
    )
    .unwrap();
    assert_eq!(
        config
            .stt()
            .routes
            .default
            .iter()
            .map(|model| model.to_request_model())
            .collect::<Vec<_>>(),
        ["platform:qq", "openai:whisper-1"]
    );
}

#[test]
fn platform_candidate_rejects_unknown_platform_model() {
    let error = parse(
        r#"
[stt]
enabled = true
[stt.routes]
default = ["platform:wechat"]
"#,
    )
    .unwrap_err();
    assert!(error.message.contains("only platform:qq"));
}

#[test]
fn platform_only_chain_satisfies_enabled_requirement() {
    let config = parse(
        r#"
[stt]
enabled = true
[stt.routes]
default = ["platform:qq"]
"#,
    )
    .unwrap();
    assert!(config.stt().enabled);
    assert_eq!(config.stt().routes.default.len(), 1);
}

#[test]
fn console_saved_shape_with_legacy_field_off_is_accepted() {
    let config = parse(
        r#"
[stt]
enabled = false
platform_asr = "off"
[stt.routes]
default = ["platform:qq", "openai:whisper-1"]
"#,
    )
    .unwrap();
    assert!(!config.stt().enabled);
    assert_eq!(
        config
            .stt()
            .routes
            .default
            .iter()
            .map(|model| model.to_request_model())
            .collect::<Vec<_>>(),
        ["platform:qq", "openai:whisper-1"]
    );
}

#[test]
fn duplicate_platform_candidate_is_rejected() {
    let error = parse(
        r#"
[stt]
enabled = true
[stt.routes]
default = ["platform:qq", "platform:qq"]
"#,
    )
    .unwrap_err();
    assert!(error.message.contains("duplicate"));
}

#[test]
fn stt_candidate_requires_explicit_provider_prefix() {
    let error = parse(
        r#"
[stt]
enabled = true
[stt.routes]
default = ["whisper-1"]
"#,
    )
    .unwrap_err();
    assert!(error.message.contains("provider:model"));
}

#[test]
fn stt_rejects_unknown_custom_provider() {
    let error = parse(
        r#"
[stt]
enabled = true
[stt.routes]
default = ["missing:whisper-1"]
"#,
    )
    .unwrap_err();
    assert!(error.message.contains("not configured"));
}

#[test]
fn stt_rejects_responses_provider() {
    let error = parse(
        r#"
[providers.custom]
kind = "openai_responses"
base_url = "https://example.test/v1"
api_key_env = "CUSTOM_API_KEY"

[stt]
enabled = true
[stt.routes]
default = ["custom:whisper-1"]
"#,
    )
    .unwrap_err();
    assert!(error.message.contains("openai_compatible"));
}

#[test]
fn stt_allows_disabled_provider_so_runtime_skips_it() {
    let config = parse(
        r#"
[providers.custom]
enabled = false
kind = "openai_compatible"
base_url = "https://example.test/v1"
api_key_env = "CUSTOM_API_KEY"

[stt]
enabled = true
[stt.routes]
default = ["custom:whisper-1"]
"#,
    )
    .unwrap();
    assert_eq!(config.stt().routes.default.len(), 1);
}

#[test]
fn stt_rejects_invalid_platform_asr_and_ranges() {
    let error = parse("[stt]\nplatform_asr = \"always\"\n").unwrap_err();
    assert!(error.message.contains("platform_asr"));

    let error = parse("[stt]\ntimeout_seconds = 0\n").unwrap_err();
    assert!(error.message.contains("timeout_seconds"));

    let error = parse("[stt]\nplatform_asr_attempts = 4\n").unwrap_err();
    assert!(error.message.contains("platform_asr_attempts"));

    let error = parse("[stt]\nmax_audio_bytes = 1\n").unwrap_err();
    assert!(error.message.contains("max_audio_bytes"));
}

#[test]
fn stt_language_rejects_whitespace() {
    let error = parse("[stt]\nlanguage = \"zh CN\"\n").unwrap_err();
    assert!(error.message.contains("language"));
}

#[test]
fn stt_duplicate_candidate_is_rejected() {
    let error = parse(
        r#"
[stt]
enabled = true
[stt.routes]
default = ["openai:whisper-1", "openai:whisper-1"]
"#,
    )
    .unwrap_err();
    assert!(error.message.contains("duplicate"));
}

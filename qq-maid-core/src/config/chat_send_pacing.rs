//! `send_message` 的拟人发送节奏配置。
//!
//! 延迟只决定工具在调用平台投递前的等待时间；真实发送结果、条数预算和去重仍由
//! 发言工具与 Gateway 负责。首条延迟会被工具剩余预算裁剪，避免为了拟人而丢消息。

use std::time::Duration;

use rand::RngExt;

use crate::error::LlmError;

pub const DEFAULT_CHAT_SEND_DELAY_MIN_MS: u64 = 1000;
pub const DEFAULT_CHAT_SEND_DELAY_MAX_MS: u64 = 2000;
pub const DEFAULT_CHAT_SEND_GAP_MS: u64 = 900;
const MAX_CHAT_SEND_DELAY_MS: u64 = 30_000;
const MAX_CHAT_SEND_GAP_MS: u64 = 10_000;

/// 工具截止时间前至少保留的真实发送预算；不足时放弃本次拟人延迟。
const SEND_DELAY_SAFETY_MARGIN: Duration = Duration::from_secs(3);

/// 语音链路包含 TTS 合成、上传和发送，需要比纯文本更长的真实发送预算。
const VOICE_SEND_SAFETY_MARGIN: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSendPacingConfig {
    pub delay_min: Duration,
    pub delay_max: Duration,
    pub gap: Duration,
}

impl Default for ChatSendPacingConfig {
    fn default() -> Self {
        Self {
            delay_min: Duration::from_millis(DEFAULT_CHAT_SEND_DELAY_MIN_MS),
            delay_max: Duration::from_millis(DEFAULT_CHAT_SEND_DELAY_MAX_MS),
            gap: Duration::from_millis(DEFAULT_CHAT_SEND_GAP_MS),
        }
    }
}

impl ChatSendPacingConfig {
    /// 测试和未注入配置时使用：不增加首条延迟，但保留条间停顿默认值。
    pub fn disabled() -> Self {
        Self {
            delay_min: Duration::ZERO,
            delay_max: Duration::ZERO,
            gap: Duration::from_millis(DEFAULT_CHAT_SEND_GAP_MS),
        }
    }

    /// 从当前有效环境读取；`0/0` 表示关闭首条延迟。
    pub fn from_environment() -> Result<Self, LlmError> {
        Self::from_lookup(super::env_optional)
    }

    /// 供配置解析和测试共用的取值入口；`lookup` 只返回已设置的原始字符串。
    pub(crate) fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, LlmError> {
        let delay_min_ms = ranged_u64(
            &lookup,
            "CHAT_SEND_DELAY_MIN_MS",
            DEFAULT_CHAT_SEND_DELAY_MIN_MS,
            0,
            MAX_CHAT_SEND_DELAY_MS,
        )?;
        let delay_max_ms = ranged_u64(
            &lookup,
            "CHAT_SEND_DELAY_MAX_MS",
            DEFAULT_CHAT_SEND_DELAY_MAX_MS,
            0,
            MAX_CHAT_SEND_DELAY_MS,
        )?;
        if delay_min_ms > delay_max_ms {
            return Err(LlmError::config(
                "CHAT_SEND_DELAY_MIN_MS must be less than or equal to CHAT_SEND_DELAY_MAX_MS",
            ));
        }
        let gap_ms = ranged_u64(
            &lookup,
            "CHAT_SEND_GAP_MS",
            DEFAULT_CHAT_SEND_GAP_MS,
            0,
            MAX_CHAT_SEND_GAP_MS,
        )?;
        Ok(Self {
            delay_min: Duration::from_millis(delay_min_ms),
            delay_max: Duration::from_millis(delay_max_ms),
            gap: Duration::from_millis(gap_ms),
        })
    }

    /// 在配置区间内随机采样一次首条延迟；区间相等时即固定值。
    pub fn sample_delay(&self) -> Duration {
        if self.delay_max <= self.delay_min {
            return self.delay_min;
        }
        let min_ms = self.delay_min.as_millis() as u64;
        let max_ms = self.delay_max.as_millis() as u64;
        Duration::from_millis(rand::rng().random_range(min_ms..=max_ms))
    }

    /// 按工具剩余预算裁剪本次延迟，至少给真实发送保留安全边际。
    pub fn effective_delay(&self, remaining: Duration) -> Duration {
        self.sample_delay()
            .min(remaining.saturating_sub(SEND_DELAY_SAFETY_MARGIN))
    }

    /// 语音的目标停顿上限。
    ///
    /// 语音先合成再发送，采样值只是总停顿目标；Gateway 会扣除 TTS 合成耗时，
    /// 只等待剩余部分。这里按语音链路预算裁剪，避免把 TTS 和上传挤超时。
    pub fn effective_voice_delay(&self, remaining: Duration) -> Duration {
        self.sample_delay()
            .min(remaining.saturating_sub(VOICE_SEND_SAFETY_MARGIN))
    }
}

fn ranged_u64(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, LlmError> {
    let Some(value) = lookup(name) else {
        return Ok(default);
    };
    let parsed = value
        .parse::<u64>()
        .map_err(|_| LlmError::config(format!("unsupported integer value for {name}: {value}")))?;
    if !(min..=max).contains(&parsed) {
        return Err(LlmError::config(format!(
            "{name} must be between {min} and {max}"
        )));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_enable_random_delay() {
        let config = ChatSendPacingConfig::default();
        assert_eq!(config.delay_min, Duration::from_millis(1000));
        assert_eq!(config.delay_max, Duration::from_millis(2000));
        assert_eq!(config.gap, Duration::from_millis(900));
    }

    #[test]
    fn fixed_range_samples_exact_value() {
        let config = ChatSendPacingConfig {
            delay_min: Duration::from_millis(1500),
            delay_max: Duration::from_millis(1500),
            gap: Duration::ZERO,
        };
        assert_eq!(config.sample_delay(), Duration::from_millis(1500));
    }

    #[test]
    fn sampled_delay_stays_in_range() {
        let config = ChatSendPacingConfig {
            delay_min: Duration::from_millis(800),
            delay_max: Duration::from_millis(1200),
            gap: Duration::ZERO,
        };
        for _ in 0..50 {
            let delay = config.sample_delay();
            assert!(delay >= Duration::from_millis(800));
            assert!(delay <= Duration::from_millis(1200));
        }
    }

    #[test]
    fn effective_delay_reserves_send_budget() {
        let config = ChatSendPacingConfig {
            delay_min: Duration::from_secs(2),
            delay_max: Duration::from_secs(2),
            gap: Duration::ZERO,
        };
        // 剩余 4 秒时只允许 1 秒，给真实发送留 3 秒。
        assert_eq!(
            config.effective_delay(Duration::from_secs(4)),
            Duration::from_secs(1)
        );
        // 剩余预算不足安全边际时直接放弃延迟。
        assert_eq!(
            config.effective_delay(Duration::from_secs(2)),
            Duration::ZERO
        );
    }

    #[test]
    fn voice_delay_reserves_tts_and_upload_budget() {
        let config = ChatSendPacingConfig {
            delay_min: Duration::from_secs(2),
            delay_max: Duration::from_secs(2),
            gap: Duration::ZERO,
        };
        // 语音链路要留 8 秒：剩余 10 秒最多等 2 秒。
        assert_eq!(
            config.effective_voice_delay(Duration::from_secs(10)),
            Duration::from_secs(2)
        );
        // 剩余 9 秒时只剩 1 秒可等。
        assert_eq!(
            config.effective_voice_delay(Duration::from_secs(9)),
            Duration::from_secs(1)
        );
        // 不足 8 秒安全边际时放弃停顿。
        assert_eq!(
            config.effective_voice_delay(Duration::from_secs(8)),
            Duration::ZERO
        );
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;
    use std::collections::HashMap;

    fn parse(pairs: &[(&str, &str)]) -> Result<ChatSendPacingConfig, LlmError> {
        let map = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect::<HashMap<_, _>>();
        ChatSendPacingConfig::from_lookup(|name| map.get(name).cloned())
    }

    #[test]
    fn unset_values_use_enabled_defaults() {
        let config = parse(&[]).unwrap();
        assert_eq!(config, ChatSendPacingConfig::default());
    }

    #[test]
    fn zero_zero_disables_initial_delay() {
        let config = parse(&[
            ("CHAT_SEND_DELAY_MIN_MS", "0"),
            ("CHAT_SEND_DELAY_MAX_MS", "0"),
            ("CHAT_SEND_GAP_MS", "0"),
        ])
        .unwrap();
        assert_eq!(config.sample_delay(), Duration::ZERO);
        assert_eq!(config.gap, Duration::ZERO);
    }

    #[test]
    fn rejects_reversed_range_and_overflow() {
        assert!(
            parse(&[
                ("CHAT_SEND_DELAY_MIN_MS", "3000"),
                ("CHAT_SEND_DELAY_MAX_MS", "1000"),
            ])
            .is_err()
        );
        assert!(parse(&[("CHAT_SEND_DELAY_MIN_MS", "30001")]).is_err());
        assert!(parse(&[("CHAT_SEND_GAP_MS", "10001")]).is_err());
    }
}

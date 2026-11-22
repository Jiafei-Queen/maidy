//! 语音时长的手工估算。
//!
//! 千问与 OpenAI Speech 都不返回音频时长，仓库也不为这一段停顿引入音频解析依赖。
//! 这里按朗读速度做保守估算，只用于拟人发送停顿，不参与 TTS、上传或平台发送判定。

use std::time::Duration;

/// 中文 TTS 正常语速约 4.5 字/秒，即每个字符约 222ms。
///
/// 标点会带来自然停顿，随字符数一起计入即可；估算只需要稳定、可解释，
/// 不需要与真实音频逐毫秒对齐。
const ESTIMATED_MS_PER_CHAR: u64 = 222;

/// QQ 单条语音的实际时长上限：普通用户 60 秒，会员更长。
///
/// Bot 富媒体文档只限制文件大小，没有写时长；但语音条一旦超过本机 QQ 的录制
/// 上限就会被截断或无法播放。这里以普通用户上限作为安全边界，估算超过它时
/// 不再生成语音，避免合成一条注定失败的音频。
pub const MAX_QQ_VOICE_DURATION: Duration = Duration::from_secs(60);

/// 按朗读文本估算语音时长；空文本为 0。
pub fn estimated_speech_duration(text: &str) -> Duration {
    let chars = text.chars().count() as u64;
    Duration::from_millis(chars.saturating_mul(ESTIMATED_MS_PER_CHAR))
}

/// 估算时长是否超过 QQ 单条语音上限。
///
/// 工具在合成前用这个判断拒绝超长文本；因此真正进入发送的语音，其估算时长
/// 天然不超过 [`MAX_QQ_VOICE_DURATION`]，用于拟人停顿的时长贡献也随之有界，
/// 不需要再单独裁剪。
pub fn exceeds_qq_voice_limit(text: &str) -> bool {
    estimated_speech_duration(text) > MAX_QQ_VOICE_DURATION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_has_no_duration() {
        assert_eq!(estimated_speech_duration(""), Duration::ZERO);
    }

    #[test]
    fn duration_scales_with_character_count() {
        assert_eq!(estimated_speech_duration("晚"), Duration::from_millis(222));
        assert_eq!(
            estimated_speech_duration("晚安"),
            Duration::from_millis(444)
        );
        // 多字节字符按 Unicode 字符计数，不按字节。
        assert_eq!(
            estimated_speech_duration("晚安呀"),
            Duration::from_millis(666)
        );
    }

    #[test]
    fn long_text_estimation_stays_proportional() {
        let text = "朗".repeat(600);
        assert_eq!(
            estimated_speech_duration(&text),
            Duration::from_millis(600 * 222)
        );
    }

    #[test]
    fn qq_voice_limit_is_sixty_seconds() {
        // 270 字 * 222ms = 59.94s，仍在 60 秒内。
        assert!(!exceeds_qq_voice_limit(&"朗".repeat(270)));
        // 271 字 * 222ms = 60.162s，超过平台可播放上限。
        assert!(exceeds_qq_voice_limit(&"朗".repeat(271)));
    }

    #[test]
    fn accepted_text_never_exceeds_platform_limit() {
        // 拒绝逻辑已保证进入发送的文本不超过上限，因此停顿贡献天然有界。
        for chars in [0, 1, 30, 270] {
            let text = "朗".repeat(chars);
            assert!(!exceeds_qq_voice_limit(&text));
            assert!(estimated_speech_duration(&text) <= MAX_QQ_VOICE_DURATION);
        }
    }
}

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

/// 按朗读文本估算语音时长；空文本为 0。
pub fn estimated_speech_duration(text: &str) -> Duration {
    let chars = text.chars().count() as u64;
    Duration::from_millis(chars.saturating_mul(ESTIMATED_MS_PER_CHAR))
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
}

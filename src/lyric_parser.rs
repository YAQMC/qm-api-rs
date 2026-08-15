//! 歌词解析: LRC 与 QRC (逐字歌词).
//!
//! - LRC: `[mm:ss.xx]` 时间标签 + 元数据标签 (`ti:/ar:/al:/by:/offset:`).
//!   对应官方桌面客户端 `lyric.js` 的 `parseLyric`.
//! - QRC: `[start_ms,end_ms]word(duration_cs)`, 逐字时间.
//!
//! 解析逻辑与官方客户端对齐; 结果可直接用于播放器高亮/滚动.

use std::collections::HashMap;

/// 歌词行.
#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    /// 时间戳 (毫秒, 已应用 offset 调整).
    pub time_ms: i64,
    /// 歌词文本.
    pub text: String,
}

/// 解析后的 LRC 歌词.
#[derive(Debug, Clone, Default)]
pub struct Lyric {
    /// 元数据 (ti/ar/al/by/offset 等).
    pub meta: HashMap<String, String>,
    /// 按时间排序的歌词行.
    pub lines: Vec<LyricLine>,
}

impl Lyric {
    /// 解析 LRC 文本.
    pub fn parse(text: &str) -> Lyric {
        parse_lrc(text)
    }

    /// 定位指定毫秒对应的歌词 (当前行).
    pub fn line_at(&self, ms: i64) -> Option<&LyricLine> {
        let mut current: Option<&LyricLine> = None;
        for line in &self.lines {
            if line.time_ms <= ms {
                current = Some(line);
            } else {
                break;
            }
        }
        current
    }

    /// 渲染回 LRC 文本.
    pub fn to_lrc(&self) -> String {
        let mut out = String::new();
        let mut pairs: Vec<(&String, &String)> = self.meta.iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        for (k, v) in pairs {
            if k != "offset" {
                out.push_str(&format!("[{k}:{v}]\n"));
            }
        }
        if let Some(off) = self.meta.get("offset") {
            out.push_str(&format!("[offset:{off}]\n"));
        }
        for line in &self.lines {
            out.push_str(&format!(
                "[{}]{}\n",
                ms_to_lrc_time(line.time_ms),
                line.text
            ));
        }
        out
    }
}

/// 将 `[mm:ss.xx]` 时间解析为毫秒 (xx 为厘秒).
fn lrc_time_to_ms(tag: &str) -> Option<i64> {
    let colon = tag.find(':')?;
    let minutes: i64 = tag[..colon].trim().parse().ok()?;
    let rest = &tag[colon + 1..];
    let dot = rest.find('.').unwrap_or(rest.len());
    let seconds: i64 = rest[..dot].trim().parse().ok()?;
    let centis: i64 = if dot < rest.len() {
        let cs = &rest[dot + 1..];
        let mut digits = String::from(cs.trim());
        digits.truncate(2);
        if digits.is_empty() {
            0
        } else {
            digits.parse().unwrap_or(0)
        }
    } else {
        0
    };
    Some(minutes * 60_000 + seconds * 1000 + centis * 10)
}

/// 将毫秒渲染为 `[mm:ss.xx]`.
pub fn ms_to_lrc_time(ms: i64) -> String {
    let minutes = ms / 60_000;
    let seconds = (ms % 60_000) / 1000;
    let centis = (ms % 1000) / 10;
    format!("{minutes:02}:{seconds:02}.{centis:02}")
}

/// 解析 LRC 文本.
pub fn parse_lrc(text: &str) -> Lyric {
    let mut meta: HashMap<String, String> = HashMap::new();
    let mut lines: Vec<LyricLine> = Vec::new();
    let mut offset: i64 = 0;
    let mut pre_time = 0i64;

    for raw in text.lines() {
        let item = raw.trim_end_matches('\r');
        let r_index = match item.rfind(']') {
            Some(i) => i,
            None => continue,
        };
        let l_substr = &item[..=r_index];
        let r_substr = item[r_index + 1..].trim().to_string();
        let tmp_times: Vec<&str> = l_substr
            .split(']')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_start_matches('['))
            .collect();

        let mut found_time = false;
        for tag in tmp_times.iter().rev() {
            let lower = tag.to_lowercase();
            if lower.starts_with("al:") {
                meta.insert("al".into(), tag[3..].to_string());
            } else if lower.starts_with("ar:") {
                meta.insert("ar".into(), tag[3..].to_string());
            } else if lower.starts_with("ti:") {
                meta.insert("ti".into(), tag[3..].to_string());
            } else if lower.starts_with("by:") {
                meta.insert("by".into(), tag[3..].to_string());
            } else if lower.starts_with("offset:") {
                if let Ok(o) = tag[7..].trim().parse::<i64>() {
                    offset = o;
                    meta.insert("offset".into(), tag[7..].trim().to_string());
                }
            } else if let Some(t) = lrc_time_to_ms(tag) {
                let adjusted = t - offset;
                let text = r_substr.replace("&apos;", "’");
                if !text.is_empty() {
                    lines.push(LyricLine {
                        time_ms: adjusted,
                        text,
                    });
                    pre_time = adjusted;
                    found_time = true;
                }
            }
        }
        // 无有效时间标签的行: 沿用上一时间.
        if !found_time && !r_substr.is_empty() {
            lines.push(LyricLine {
                time_ms: pre_time,
                text: r_substr,
            });
        }
    }

    lines.sort_by_key(|l| l.time_ms);
    Lyric { meta, lines }
}

/// QRC 逐字歌词词条.
#[derive(Debug, Clone, PartialEq)]
pub struct QrcWord {
    /// 文本.
    pub text: String,
    /// 起始毫秒.
    pub start_ms: i64,
    /// 持续毫秒.
    pub duration_ms: i64,
}

/// QRC 逐字歌词行.
#[derive(Debug, Clone, PartialEq)]
pub struct QrcLine {
    /// 行起始毫秒.
    pub start_ms: i64,
    /// 行结束毫秒.
    pub end_ms: i64,
    /// 该行文本.
    pub text: String,
    /// 逐字词条.
    pub words: Vec<QrcWord>,
}

/// 解析后的 QRC 逐字歌词.
#[derive(Debug, Clone, Default)]
pub struct QrcLyric {
    pub offset: i64,
    pub lines: Vec<QrcLine>,
}

impl QrcLyric {
    /// 解析 QRC 文本 (仅当歌词接口 `qrc=true` 时返回).
    ///
    /// 格式: `[offset:N]` 元数据 + `[start_ms,end_ms]word(dur_cs)word(dur_cs)...`
    /// 字持续时长单位为厘秒 (×10 得到毫秒).
    pub fn parse(text: &str) -> QrcLyric {
        parse_qrc(text)
    }

    /// 定位指定毫秒对应的行.
    pub fn line_at(&self, ms: i64) -> Option<&QrcLine> {
        let mut current: Option<&QrcLine> = None;
        for line in &self.lines {
            if line.start_ms <= ms {
                current = Some(line);
            } else {
                break;
            }
        }
        current
    }
}

/// 解析 QRC 逐字歌词.
pub fn parse_qrc(text: &str) -> QrcLyric {
    let mut offset = 0i64;
    let mut lines: Vec<QrcLine> = Vec::new();

    for raw in text.lines() {
        let item = raw.trim_end_matches('\r').trim();
        if item.is_empty() {
            continue;
        }
        // offset 元数据.
        if let Some(rest) = item.strip_prefix("[offset:") {
            if let Some(v) = rest.strip_suffix(']') {
                if let Ok(o) = v.trim().parse::<i64>() {
                    offset = o;
                }
            }
            continue;
        }
        // [start,end]...
        if let Some(rest) = item.strip_prefix('[') {
            let close = match rest.find(']') {
                Some(i) => i,
                None => continue,
            };
            let range = &rest[..close];
            let mut parts = range.split(',');
            let start: i64 = parts
                .next()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            let end: i64 = parts
                .next()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(start);
            let body = &rest[close + 1..];

            // 解析 word(dur) 序列.
            let mut words = Vec::new();
            let mut pos = 0usize;
            let mut cursor_ms = start;
            let mut line_text = String::new();
            while pos < body.len() {
                let open = match body[pos..].find('(') {
                    Some(i) => pos + i,
                    None => break,
                };
                let word = &body[pos..open];
                let close_p = match body[open..].find(')') {
                    Some(i) => open + i,
                    None => break,
                };
                let inner = &body[open + 1..close_p];
                let dur: i64 = inner
                    .split(',')
                    .next()
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(0);
                let dur_ms = dur * 10;
                if !word.is_empty() {
                    line_text.push_str(word);
                    words.push(QrcWord {
                        text: word.to_string(),
                        start_ms: cursor_ms - offset,
                        duration_ms: dur_ms,
                    });
                }
                cursor_ms += dur_ms;
                pos = close_p + 1;
            }
            if !line_text.is_empty() {
                lines.push(QrcLine {
                    start_ms: start - offset,
                    end_ms: end - offset,
                    text: line_text,
                    words,
                });
            }
        }
    }

    lines.sort_by_key(|l| l.start_ms);
    QrcLyric { offset, lines }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_basic() {
        let text = "[ti:晴天]\n[ar:周杰伦]\n[00:00.00]晴天 - 周杰伦\n[00:02.25]词：周杰伦\n[00:06.75]编曲\n";
        let lyric = parse_lrc(text);
        assert_eq!(lyric.meta.get("ti").map(|s| s.as_str()), Some("晴天"));
        assert_eq!(lyric.meta.get("ar").map(|s| s.as_str()), Some("周杰伦"));
        assert_eq!(lyric.lines.len(), 3);
        assert_eq!(lyric.lines[0].time_ms, 0);
        assert_eq!(lyric.lines[1].time_ms, 2250);
        assert_eq!(lyric.lines[2].time_ms, 6750);
        assert_eq!(
            lyric.line_at(3000).map(|l| l.text.as_str()),
            Some("词：周杰伦")
        );
    }

    #[test]
    fn lrc_offset() {
        let text = "[offset:500]\n[00:01.00]你好\n[00:02.00]世界\n";
        let lyric = parse_lrc(text);
        // offset 500ms 被减去.
        assert_eq!(lyric.lines[0].time_ms, 500);
        assert_eq!(lyric.lines[1].time_ms, 1500);
    }

    #[test]
    fn lrc_multiple_tags() {
        let text = "[00:01.00][00:05.00]重复\n";
        let lyric = parse_lrc(text);
        assert_eq!(lyric.lines.len(), 2);
        assert_eq!(lyric.lines[0].time_ms, 1000);
        assert_eq!(lyric.lines[1].time_ms, 5000);
    }

    #[test]
    fn qrc_basic() {
        let text = "[offset:0]\n[0,1800]举(300)起(300)双(300)手(300)放(300)开(300)\n";
        let qrc = parse_qrc(text);
        assert_eq!(qrc.lines.len(), 1);
        let line = &qrc.lines[0];
        assert_eq!(line.start_ms, 0);
        assert_eq!(line.end_ms, 1800);
        assert_eq!(line.text, "举起双手放开");
        assert_eq!(line.words.len(), 6);
        // 字持续单位为厘秒 → ×10 ms.
        assert_eq!(line.words[0].start_ms, 0);
        assert_eq!(line.words[0].duration_ms, 3000);
        assert_eq!(line.words[1].start_ms, 3000);
        assert_eq!(line.words[5].start_ms, 15000);
    }

    #[test]
    fn ms_roundtrip() {
        assert_eq!(ms_to_lrc_time(2250), "00:02.25");
        assert_eq!(lrc_time_to_ms("00:02.25"), Some(2250));
        assert_eq!(lrc_time_to_ms("01:30"), Some(90_000));
    }
}

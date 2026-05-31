//! 从 PTY stdin 原始字节中剥离终端控制序列，再按行编辑语义提取用户输入。

const MAX_PROMPT_CHARS: usize = 4_096;
const MAX_PROMPT_LINES: usize = 4;

/// 剥离 CSI / OSC / DCS 等 VT 序列，保留可打印字符与行编辑相关控制符。
/// 增强版：额外识别 bracketed paste（ESC[200~ ... ESC[201~）区间，完整保留 paste 内容用于后续精确提取。
pub fn strip_terminal_escapes(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == 0x1b {
            i += 1;
            if i >= input.len() {
                break;
            }
            match input[i] {
                b'[' => {
                    i += 1;
                    // 特殊处理 bracketed paste 开始/结束标记
                    if i + 3 < input.len()
                        && input[i] == b'2'
                        && input[i + 1] == b'0'
                        && input[i + 2] == b'0'
                        && input[i + 3] == b'~'
                    {
                        // 跳过开始标记本身，不输出到 cleaned（paste 内容后续单独处理）
                        i += 4;
                        continue;
                    }
                    if i + 3 < input.len()
                        && input[i] == b'2'
                        && input[i + 1] == b'0'
                        && input[i + 2] == b'1'
                        && input[i + 3] == b'~'
                    {
                        i += 4;
                        continue;
                    }
                    while i < input.len() && !(0x40..=0x7e).contains(&input[i]) {
                        i += 1;
                    }
                    if i < input.len() {
                        i += 1;
                    }
                }
                b']' => {
                    i += 1;
                    while i < input.len() {
                        if input[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if input[i] == 0x1b && i + 1 < input.len() && input[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                b'P' => {
                    i += 1;
                    while i < input.len() {
                        if input[i] == 0x1b && i + 1 < input.len() && input[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => i += 1,
            }
            continue;
        }

        let b = input[i];
        if b.is_ascii_control() && b != b'\r' && b != b'\n' && b != b'\t' && b != 0x7f && b != 0x08
        {
            i += 1;
            continue;
        }
        out.push(b);
        i += 1;
    }
    out
}

pub fn parse_submitted_lines(cleaned: &[u8]) -> Vec<String> {
    let mut current = Vec::new();
    let mut prompts = Vec::new();

    for &byte in cleaned {
        match byte {
            b'\x7f' | b'\x08' => pop_utf8_char(&mut current),
            b'\r' | b'\n' => {
                if let Some(text) = normalize_prompt(&decode_utf8(&current)) {
                    prompts.push(text);
                }
                current.clear();
            }
            b if b.is_ascii_control() => {}
            b => current.push(b),
        }
    }

    if let Some(text) = normalize_prompt(&decode_utf8(&current)) {
        prompts.push(text);
    }

    prompts
}

/// 带原始字节偏移的 stdin 提交记录（专为时间轴 / 完整输入还原设计）。
///
/// 与旧的 parse_submitted_lines 不同：
/// - 直接在原始 stdin 上工作，能准确定位 bracketed paste 区间
/// - 对 paste 内容更宽松（支持多行、较短文本、内部包含的控制字符被正确剥离）
/// - 提供 raw_start / raw_end，方便后续与 stdout / events 做时间对齐和区间标注
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSubmission {
    pub text: String,
    /// 在原始 stdin 字节流中的起始偏移（包含 bracketed paste 标记本身）
    pub raw_start: usize,
    /// 在原始 stdin 字节流中的结束偏移（不含）
    pub raw_end: usize,
    /// 是否来自 bracketed paste（paste 通常是用户一次完整粘贴的多行输入）
    pub from_bracketed_paste: bool,
}

/// 从原始 stdin 字节流中提取提交记录（支持 bracketed paste + 基本行编辑）。
///
/// 这是实现“综合 stdout + stdin + events 完整还原输入”的核心解析器。
/// 它不依赖 vt100 屏幕匹配，仅靠 stdin 自身语义 + bracketed paste 标记。
pub fn extract_raw_submissions(raw: &[u8]) -> Vec<RawSubmission> {
    let mut submissions = Vec::new();
    let mut i = 0usize;

    while i < raw.len() {
        // 检测 bracketed paste 开始：ESC [ 2 0 0 ~
        if raw[i] == 0x1b && i + 5 < raw.len() && &raw[i..i + 6] == b"\x1b[200~" {
            let paste_start = i; // 包含起始标记
            i += 6;
            let content_start = i;

            // 寻找结束标记 ESC [ 2 0 1 ~
            while i + 5 < raw.len() && &raw[i..i + 6] != b"\x1b[201~" {
                i += 1;
            }

            let content_end = i;
            if i + 5 < raw.len() {
                i += 6; // 跳过结束标记
            }

            let paste_content_raw = &raw[content_start..content_end];
            // 对 paste 内部也做一次转义剥离（极少数情况 paste 里还混了其他 CSI）
            let cleaned = strip_terminal_escapes(paste_content_raw);
            let text = decode_utf8(&cleaned)
                .trim_end_matches(['\r', '\n'])
                .to_string();

            if !text.trim().is_empty() {
                submissions.push(RawSubmission {
                    text,
                    raw_start: paste_start,
                    raw_end: i.min(raw.len()),
                    from_bracketed_paste: true,
                });
            }
            continue;
        }

        // 普通输入路径：模拟简单行编辑 + 提交
        let commit_start = i;
        let mut current: Vec<u8> = Vec::new();

        while i < raw.len() {
            let b = raw[i];

            if b == 0x1b {
                // 可能是其他 CSI（箭头、Delete 等），先跳过整个 CSI，暂不完整模拟光标移动
                // （完整模拟需要状态机，后续可增强；当前先保证不丢内容）
                i += 1;
                if i < raw.len() && raw[i] == b'[' {
                    i += 1;
                    while i < raw.len() && !(0x40..=0x7e).contains(&raw[i]) {
                        i += 1;
                    }
                    if i < raw.len() {
                        i += 1;
                    }
                    continue;
                }
                // 其他 ESC，跳过
                i += 1;
                continue;
            }

            match b {
                b'\x7f' | b'\x08' => {
                    pop_utf8_char(&mut current);
                    i += 1;
                }
                b'\r' | b'\n' => {
                    let text = decode_utf8(&current);
                    let trimmed = text.trim();
                    // 放宽策略：只要有可读内容就记录（timeline 视图需要完整性）
                    if !trimmed.is_empty()
                        && (trimmed
                            .chars()
                            .any(|c| c.is_alphabetic() || ('\u{4e00}'..='\u{9fff}').contains(&c))
                            || trimmed.len() >= 2)
                    {
                        // 避免把纯噪声（如只剩方向键产生的空提交）计入
                        if !looks_like_raw_terminal_noise(trimmed) {
                            submissions.push(RawSubmission {
                                text: trimmed.to_string(),
                                raw_start: commit_start,
                                raw_end: i + 1,
                                from_bracketed_paste: false,
                            });
                        }
                    }
                    current.clear();
                    i += 1;
                    break;
                }
                // 其他控制符（方向键 CSI 已在上面处理）
                b if b.is_ascii_control() => {
                    i += 1;
                }
                _ => {
                    current.push(b);
                    i += 1;
                }
            }
        }

        // 尾部未提交的缓冲（会话结束时用户可能还在输入但没按回车，或最后一次提交后还有内容）
        if !current.is_empty() {
            let text = decode_utf8(&current).trim().to_string();
            if !text.is_empty()
                && (text
                    .chars()
                    .any(|c| c.is_alphabetic() || ('\u{4e00}'..='\u{9fff}').contains(&c))
                    || text.len() >= 2)
                && !looks_like_raw_terminal_noise(&text)
            {
                submissions.push(RawSubmission {
                    text,
                    raw_start: commit_start,
                    raw_end: i,
                    from_bracketed_paste: false,
                });
            }
        }
    }

    submissions
}

/// 针对 raw timeline 的宽松噪音判断（比旧的 looks_like_terminal_noise 更宽松）。
/// 只过滤极明显的 TUI 内部回显 / OSC 片段，保留用户真实输入尝试。
fn looks_like_raw_terminal_noise(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("rgb:") || lower.contains("]11;") || lower.starts_with("$ ")
}

fn decode_utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn pop_utf8_char(bytes: &mut Vec<u8>) {
    if bytes.is_empty() {
        return;
    }
    loop {
        bytes.pop();
        if bytes.is_empty() || std::str::from_utf8(bytes).is_ok() {
            break;
        }
    }
}

fn normalize_prompt(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.chars().count() < 3 {
        return None;
    }
    if text.chars().count() > MAX_PROMPT_CHARS {
        return None;
    }
    if text.lines().count() > MAX_PROMPT_LINES {
        return None;
    }
    if looks_like_terminal_noise(text) {
        return None;
    }
    if !text
        .chars()
        .any(|c| c.is_alphabetic() || ('\u{4e00}'..='\u{9fff}').contains(&c))
    {
        return None;
    }
    Some(text.to_string())
}

fn looks_like_terminal_noise(text: &str) -> bool {
    let lower = text.to_lowercase();

    // 始终拒绝的强信号（不论长短）
    if lower.contains("rgb:")
        || lower.contains("]11;")
        || lower.contains("globbing")
        || lower.contains("grepping")
        || lower.contains("composer")
        || lower.starts_with("$ ")
        || text.starts_with("→ ")
        || lower.contains("monitored background")
        || lower.contains("waited for")
        || lower.contains("complete|error")
        || lower.contains("complete|")
        || lower.contains("\"status\"")
        || lower.contains("\"conclusion\"")
        || lower.contains("conclusion")
        || lower.contains("in_progress")
        || lower.contains("startedat")
        || lower.contains("/actions/r")
        || lower.contains("grepped")
        || lower.contains("globbed")
        || lower.starts_with("on branch")
        || lower.starts_with("your br")
        || lower.contains("to-do")
        || lower.contains("user (")
        || lower.contains("human (")
        || lower.trim_start().starts_with("user ")
        || lower.trim_start().starts_with("human ")
    {
        return true;
    }

    // 看起来像 JSON / CI 对象 / 大段状态输出（常见于 agent 内部日志）
    if (lower.trim_start().starts_with(['{', '['])
        && (lower.contains("conclusion")
            || lower.contains("status")
            || lower.contains("startedat")
            || lower.contains("jobs")))
        || (lower.contains(',')
            && lower.matches(',').count() >= 3
            && (lower.contains("status") || lower.contains("conclusion")))
    {
        return true;
    }

    // 数字开头 + 包含 background / 典型 CI 片段（放宽长度限制）
    if lower
        .trim_start()
        .starts_with(['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '{'])
        && lower.contains("background")
    {
        return true;
    }

    // 明显是之前渲染历史里的标签或时间戳（Cursor/Claude 常见）
    if lower.contains("user (") || lower.contains("human (") || lower.contains("agent (") {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_focus_and_osc_from_stdin() {
        let raw = b"\x1b]11;rgb:ffff/fcfc/f0f0\x07\x1b[Is\x7fe\x7f\x1b]11;rgb:ffff/fcfc/f0f0\x07\
            \xe8\xbf\x99\xe4\xb8\xaa\xe9\xa1\xb9\xe7\x9b\xae\xe8\xbf\x98\xe6\x9c\x89\xe4\xbb\x80\xe4\xb9\x88\
            \xe4\xbc\x98\xe5\x8c\x96\xe7\x9a\x84\xe5\x9c\xb0\xe6\x96\xb9\xe5\x90\x97\xef\xbc\x9f\n";
        let cleaned = strip_terminal_escapes(raw);
        let prompts = parse_submitted_lines(&cleaned);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0], "这个项目还有什么优化的地方吗？");
    }
}

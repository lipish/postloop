//! 从 PTY stdin 原始字节中剥离终端控制序列，再按行编辑语义提取用户输入。

const MAX_PROMPT_CHARS: usize = 4_096;
const MAX_PROMPT_LINES: usize = 4;

/// 剥离 CSI / OSC / DCS 等 VT 序列，保留可打印字符与行编辑相关控制符。
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

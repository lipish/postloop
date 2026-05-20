//! Agent 无关的终端/TUI 噪音过滤与正文行识别。

pub fn is_noise_line(line: &str) -> bool {
    is_chrome_line(line) || is_tool_status_line(line)
}

pub fn is_substantive_line(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 6 {
        return false;
    }
    if is_noise_line(t) {
        return false;
    }

    if t.starts_with('•')
        || t.starts_with('-')
        || t.starts_with('*')
        || t.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        return letter_ratio(t) >= 0.15;
    }

    letter_ratio(t) >= 0.25 || t.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

pub fn filter_content_lines(lines: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in lines {
        let t = line.trim().to_string();
        if !is_substantive_line(&t) {
            continue;
        }
        if seen.insert(t.clone()) {
            out.push(t);
        }
    }
    out
}

fn letter_ratio(text: &str) -> f64 {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 0.0;
    }
    let letters = chars
        .iter()
        .filter(|c| c.is_alphabetic() || ('\u{4e00}'..='\u{9fff}').contains(c))
        .count();
    letters as f64 / chars.len() as f64
}

pub fn is_chrome_line(line: &str) -> bool {
    if line.chars().any(|c| "▄▀─│⠀░▒▓".contains(c)) {
        return true;
    }
    if line.chars().any(|c| ('\u{2800}'..='\u{28ff}').contains(&c)) {
        return true;
    }

    let lower = line.trim().to_lowercase();
    if lower.is_empty() {
        return true;
    }

    lower.starts_with("→ ")
        || lower.contains("ctrl+c")
        || lower.contains("press ctrl+c")
        || lower.contains("to resume this session")
        || lower.contains("auto-run")
        || lower.contains("add a follow-up")
        || lower.starts_with("tip:")
        || lower.starts_with("use /")
        || lower.contains(" earlier items hidden")
        || (line.starts_with("~/") && line.len() < 100)
        || (line.contains(" · ") && line.len() < 100)
        || lower.ends_with(" tokens")
        || lower.ends_with(" token")
        || lower.ends_with("ms")
        || lower.ends_with("s") && line.contains(" | ")
}

fn is_tool_status_line(line: &str) -> bool {
    let t = line.trim();
    let lower = t.to_lowercase();

    if t.starts_with("$ ") {
        return true;
    }
    if lower.starts_with("--> ") || lower.starts_with("warning:") || lower.starts_with("error:") {
        return true;
    }
    if t.len() < 80 && lower.contains('%') && !t.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
        return true;
    }

    const TOOL_PREFIXES: &[&str] = &[
        "glob ", "globbing ", "globbed ",
        "grep ", "grepping ", "grepped ",
        "read ", "reading ", "read ",
        "run ", "running ", "ran ",
        "search ", "searching ", "searched ",
        "found ", "fetch ", "fetching ", "fetched ",
        "write ", "writing ", "wrote ",
        "list ", "listing ", "listed ",
        "plan ", "planning ", "planned ",
        "build ", "building ", "built ",
        "execute ", "executing ", "executed ",
        "call ", "calling ", "called ",
    ];

    for prefix in TOOL_PREFIXES {
        if lower.starts_with(prefix) {
            return true;
        }
    }

    if t.ends_with(" in .") || t.ends_with(" in ./") {
        return true;
    }

    // 单行文件路径 / 工具计数
    if t.starts_with("Read ") && t.len() < 80 {
        return true;
    }
    if lower.starts_with("found ") && t.len() < 60 {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_tool_lines() {
        assert!(is_noise_line("Globbing \"**/*\" in ."));
        assert!(is_noise_line("$ cd /tmp && cargo test"));
        assert!(is_noise_line("⠰⠰ Globbing  83 tokens"));
    }

    #[test]
    fn keeps_prose_lines() {
        assert!(is_substantive_line(
            "可以从代码现状看，优化空间主要在以下几块，按优先级大致如下。"
        ));
        assert!(is_substantive_line("• 版本：0.2.0，cargo build 可成功编译"));
    }
}

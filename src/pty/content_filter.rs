//! Agent 无关的终端/TUI 噪音过滤与正文行识别。
//!
//! 设计原则（抗脆弱）：
//! - 优先“正向信号”判断正文（含 CJK、长句、常见 prose 词、句末标点）。
//! - 仅当“疑似工具状态行”且“不像自然语言”时才过滤，避免误杀 "I plan to read..." 这类 prose。
//! - 工具前缀列表收紧为 Cursor/Claude 真实日志高频模式 + 长度/上下文守卫。
//! - 新 Agent TUI 出现时，优先扩展 looks_like_natural_language() 而非盲目加前缀。

pub fn is_noise_line(line: &str) -> bool {
    is_chrome_line(line) || is_tool_status_line(line) || is_agent_internal_trace(line)
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

    // 强 UI chrome 信号（几乎不可能是正文）
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
        // 仅当短状态栏形式才视为 chrome（避免误杀 "took 120ms to finish."）
        || (lower.ends_with("ms") && line.len() < 60)
        || (lower.ends_with("s") && line.contains(" | ") && line.len() < 80)
}

fn is_tool_status_line(line: &str) -> bool {
    let t = line.trim();
    let lower = t.to_lowercase();

    if t.starts_with("$ ") {
        return true;
    }
    // warning:/error: 仅短工具日志或编译输出视为噪音；长解释句保留
    if (lower.starts_with("--> ") || lower.starts_with("warning:") || lower.starts_with("error:"))
        && t.len() < 90
        && !looks_like_natural_language(&lower, t)
    {
        return true;
    }
    if t.len() < 80
        && lower.contains('%')
        && !t.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
    {
        return true;
    }

    // 强工具动词前缀（Cursor/Claude 真实状态行，几乎不出现在正文）
    const STRONG_TOOL_PREFIXES: &[&str] = &[
        "globbing ",
        "grepping ",
        "reading ",
        "writing ",
        "executing ",
        "searching ",
        "fetching ",
        "planning ",
        "building ",
    ];
    for p in STRONG_TOOL_PREFIXES {
        if lower.starts_with(p) {
            return true;
        }
    }

    // 较弱前缀 + “像 Cursor 工具日志” 的守卫（路径、in .、短 + 无 prose 词）
    if (lower.starts_with("glob ") || lower.starts_with("grep ") || lower.starts_with("run "))
        && (t.ends_with(" in .")
            || t.ends_with(" in ./")
            || lower.contains('/')
            || lower.contains('.'))
        && !looks_like_natural_language(&lower, t)
    {
        return true;
    }

    if lower.starts_with("read ")
        || lower.starts_with("write ")
        || lower.starts_with("list ")
        || lower.starts_with("execute ")
        || lower.starts_with("call ")
    {
        // 典型 Cursor "Read foo.rs" / "Write bar.py" 单行状态
        if (t.len() < 80 && (lower.contains('/') || lower.contains('.') || t.ends_with(" in .")))
            && !looks_like_natural_language(&lower, t)
        {
            return true;
        }
    }

    // "found N files" 类短计数；避免误杀 "I found that..."
    if lower.starts_with("found ")
        && t.len() < 60
        && !lower.contains(" the ")
        && !lower.contains(" that ")
    {
        return true;
    }

    // 典型 Cursor 尾缀
    if t.ends_with(" in .") || t.ends_with(" in ./") {
        return true;
    }

    // 标题式 "Read src/xx.rs"（无 prose 词）
    if t.starts_with("Read ") && t.len() < 80 && !looks_like_natural_language(&lower, t) {
        return true;
    }

    false
}

/// 正向启发式：判断该行“更像自然语言解释/思考”而非工具状态。
/// 命中则不视为噪音，保护 "I plan to read the file..." 这类句子。
/// 注意：路径中的 "." 不应触发（如 "Read foo.rs"），只认句末标点或 ". " 序列。
fn looks_like_natural_language(lower: &str, orig: &str) -> bool {
    lower.contains(" i ")
        || lower.contains(" we ")
        || lower.contains(" you ")
        || lower.contains(" the ")
        || lower.contains(" this ")
        || lower.contains(" that ")
        || lower.contains(" because ")
        || lower.contains(" 由于 ")
        || lower.contains(" 因此 ")
        || lower.contains(" important")
        || lower.contains("注意")
        || lower.contains("关键")
        || orig.contains(". ")
        || orig.contains("。")
        || orig.contains("！")
        || orig.contains("？")
        || (orig.ends_with('.') && !orig.contains('/') && !orig.contains('\\') && orig.len() > 12)
        || orig.len() > 85
}

/// 更激进的 Agent 内部思考/工具轨迹识别（专为 il dump chat 折叠设计）。
/// 匹配 Cursor、Grok Build、Claude 等 Agent 运行时产生的大量状态日志、待办列表、背景任务监控等。
/// 这些行极少是最终用户可见的“回答”，而是推理轨迹。
fn is_agent_internal_trace(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_lowercase();

    // 版本/品牌横幅
    if lower.contains("grok build")
        || lower.contains("cursor agent")
        || lower.starts_with("grok build 0.")
    {
        return true;
    }

    // To-do 列表（Agent 自我任务跟踪）
    if lower.contains("to-do working on")
        || (lower.contains("working on") && (lower.contains("to-do") || lower.contains("todo")))
    {
        return true;
    }
    if t.contains('☐') || t.contains('☒') {
        return true;
    }

    // Shell/背景任务等待监控（常见于 agent 工具循环）
    if lower.contains("waiting ") && (lower.contains("for shell") || lower.contains("background")) {
        return true;
    }
    if lower.contains("monitored background task") {
        return true;
    }

    // 典型的 Cursor/Grok 复合状态行（一次输出多动作）
    if lower.starts_with("globbing, grepping") || lower.contains("globbing, grepping, reading") {
        return true;
    }

    // jq / gh / eval 调试痕迹
    if lower.contains("select(.event")
        || lower.contains("(eval):")
        || lower.contains("no matches found")
    {
        return true;
    }

    // 网络/发布相关短状态（WebFetch、gh release、gh run 等）
    if (lower.contains("webfetch")
        || lower.contains("gh run")
        || lower.contains("gh release")
        || lower.contains("gh api"))
        && t.len() < 120
    {
        return true;
    }

    // 高密度动作动词行：多次 glob/grep/read/grepped 组合，通常是内部循环
    let verbs = [
        "globbing", "grepping", "reading", "grepped", "globbed", "ran ", "cat ",
    ];
    let count = verbs.iter().filter(|&&v| lower.contains(v)).count();
    if count >= 2 && t.len() < 220 {
        return true;
    }

    // 纯 JSON 状态对象（常出现在 agent 自我调试输出中）
    if t.starts_with('{')
        && t.ends_with('}')
        && (lower.contains("\"status\"")
            || lower.contains("\"conclusion\"")
            || lower.contains("\"in_progress\""))
    {
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
        assert!(is_noise_line("Reading src/main.rs (120 tokens)"));
        assert!(is_noise_line("Executing `cargo test` in ."));
    }

    #[test]
    fn keeps_prose_lines() {
        assert!(is_substantive_line(
            "可以从代码现状看，优化空间主要在以下几块，按优先级大致如下。"
        ));
        assert!(is_substantive_line("• 版本：0.2.0，cargo build 可成功编译"));
    }

    #[test]
    fn does_not_filter_natural_language_with_tool_verbs() {
        // 之前脆弱规则会误杀这些正文
        assert!(is_substantive_line(
            "I plan to read the requirements and refactor the glob logic."
        ));
        assert!(is_substantive_line(
            "We are reading the design doc to understand the intent."
        ));
        assert!(is_substantive_line(
            "The main issue is that the current filter is too aggressive."
        ));
        assert!(is_substantive_line(
            "Found the root cause after carefully reading the source."
        ));
        assert!(is_substantive_line(
            "Warning: this change may affect downstream users, but it is intentional."
        ));
        assert!(is_substantive_line(
            "我计划先读取 INTENT.md 再决定重构策略。"
        ));
    }

    #[test]
    fn still_filters_real_tool_status() {
        assert!(is_noise_line("Reading src/pty/content_filter.rs"));
        assert!(is_noise_line("Globbing \"**/*\" in ./src"));
        assert!(is_noise_line("Found 17 files"));
        assert!(is_noise_line("Read .cursor/rules.md"));
    }

    #[test]
    fn handles_mixed_and_edge_cases() {
        // 带 % 的进度但有中文 -> 保留（is_substantive 里的 CJK 优先）
        assert!(is_substantive_line(
            "已完成 87% 的重构工作，剩余 3 个模块。"
        ));
        // 纯英文短进度无 CJK -> 过滤
        assert!(is_noise_line("Processing 87%..."));
    }
}

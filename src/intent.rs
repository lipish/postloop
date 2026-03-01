use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct IntentInfo {
    pub id: String,
    pub title: String,
    pub content: String,
}

pub fn load_intent(repo_root: &Path) -> IntentInfo {
    let intent_path = repo_root.join("INTENT.md");
    let default_intent = IntentInfo {
        id: "intent-unknown".to_string(),
        title: "Untitled Intent".to_string(),
        content: String::new(),
    };

    let Ok(content) = fs::read_to_string(intent_path) else {
        return default_intent;
    };

    let mut id: Option<String> = None;
    let mut title: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        if id.is_none() && trimmed.starts_with("id:") {
            id = trimmed
                .split_once(':')
                .map(|(_, value)| value.trim().trim_matches('"').to_string())
                .filter(|value| !value.is_empty());
        }

        if title.is_none() && trimmed.starts_with("title:") {
            title = trimmed
                .split_once(':')
                .map(|(_, value)| value.trim().trim_matches('"').to_string())
                .filter(|value| !value.is_empty());
        }

        if id.is_some() && title.is_some() {
            break;
        }
    }

    IntentInfo {
        id: id.unwrap_or_else(|| "intent-unknown".to_string()),
        title: title.unwrap_or_else(|| "Untitled Intent".to_string()),
        content,
    }
}

pub fn build_copilot_prompt(intent: &IntentInfo) -> String {
    let body = intent
        .content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("id:") && !trimmed.starts_with("title:")
        })
        .take(24)
        .collect::<Vec<_>>()
        .join("\n");

    if body.is_empty() {
        return format!(
            "You are GitHub Copilot CLI. Implement this intent in the current repository and explain key changes briefly. Intent title: {}",
            intent.title
        );
    }

    format!(
        "You are GitHub Copilot CLI. Implement this intent in the current repository and explain key changes briefly.\n\nIntent ID: {}\nIntent Title: {}\n\nIntent Content:\n{}",
        intent.id, intent.title, body
    )
}

#[cfg(feature = "zene")]
pub fn build_intent_prompt(intent: &IntentInfo, agent_name: &str) -> String {
    let body = intent
        .content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("id:") && !trimmed.starts_with("title:")
        })
        .collect::<Vec<_>>()
        .join("\n");

    if body.is_empty() {
        return format!(
            "You are {}. Execute the user's intent in the current repository, and provide a concise final summary.",
            agent_name
        );
    }

    format!(
        "You are {}. Execute the user's intent in the current repository.\n\nIntent ID: {}\nIntent Title: {}\n\nIntent Content:\n{}\n\nWhen complete, provide a concise final summary.",
        agent_name, intent.id, intent.title, body
    )
}

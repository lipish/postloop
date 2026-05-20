pub fn make_pty_session_id(intent_id: &str, run_id: &str) -> String {
    format!("{}:{}", intent_id, run_id)
}

pub fn parse_pty_session_id(id: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = id.split(':').collect();
    if parts.len() == 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

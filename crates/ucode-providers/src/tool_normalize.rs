//! Tool name normalization for Anthropic OAuth.
//!
//! Anthropic's OAuth validation requires tool names in PascalCase.
//! Known tool names have explicit mappings; unknown names are converted
//! from snake_case to PascalCase.

/// Normalize a tool name to PascalCase for Anthropic OAuth.
///
/// Known mappings (Claude Code built-in tools):
/// - bash -> Bash
/// - read -> Read
/// - edit -> Edit
/// - write -> Write
/// - glob -> Glob
/// - grep -> Grep
/// - webfetch -> WebFetch
/// - websearch -> WebSearch
/// - task -> Task
/// - todowrite -> TodoWrite
///
/// Unknown tools: convert snake_case to PascalCase (e.g., my_tool -> MyTool).
pub fn normalize_tool_name(name: &str) -> String {
    match name {
        "bash" => "Bash".into(),
        "read" => "Read".into(),
        "edit" => "Edit".into(),
        "write" => "Write".into(),
        "glob" => "Glob".into(),
        "grep" => "Grep".into(),
        "webfetch" => "WebFetch".into(),
        "websearch" => "WebSearch".into(),
        "task" => "Task".into(),
        "todowrite" => "TodoWrite".into(),
        other => snake_to_pascal(other),
    }
}

/// Convert a snake_case string to PascalCase.
fn snake_to_pascal(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    // If no underscores and first char is uppercase, assume already PascalCase
    if !s.contains('_') && s.starts_with(|c: char| c.is_uppercase()) {
        return s.to_owned();
    }

    // If no underscores but all lowercase, just capitalize first letter
    if !s.contains('_') {
        let mut chars = s.chars();
        return match chars.next() {
            Some(c) => {
                let upper: String = c.to_uppercase().collect();
                upper + chars.as_str()
            }
            None => String::new(),
        };
    }

    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tool_names() {
        assert_eq!(normalize_tool_name("bash"), "Bash");
        assert_eq!(normalize_tool_name("read"), "Read");
        assert_eq!(normalize_tool_name("edit"), "Edit");
        assert_eq!(normalize_tool_name("write"), "Write");
        assert_eq!(normalize_tool_name("glob"), "Glob");
        assert_eq!(normalize_tool_name("grep"), "Grep");
        assert_eq!(normalize_tool_name("webfetch"), "WebFetch");
        assert_eq!(normalize_tool_name("websearch"), "WebSearch");
        assert_eq!(normalize_tool_name("task"), "Task");
        assert_eq!(normalize_tool_name("todowrite"), "TodoWrite");
    }

    #[test]
    fn snake_case_conversion() {
        assert_eq!(normalize_tool_name("my_tool"), "MyTool");
        assert_eq!(normalize_tool_name("get_weather_data"), "GetWeatherData");
    }

    #[test]
    fn already_pascal_case() {
        assert_eq!(normalize_tool_name("MyTool"), "MyTool");
        assert_eq!(normalize_tool_name("GetWeather"), "GetWeather");
    }

    #[test]
    fn single_word_lowercase() {
        assert_eq!(normalize_tool_name("search"), "Search");
    }

    #[test]
    fn empty_string() {
        assert_eq!(normalize_tool_name(""), "");
    }

    #[test]
    fn underscores_only() {
        assert_eq!(normalize_tool_name("___"), "");
    }
}

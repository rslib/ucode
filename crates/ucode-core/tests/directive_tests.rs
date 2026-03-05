use ucode_core::{Directive, Span, parse_input};

fn text(content: &str, start: usize, end: usize) -> Directive {
    Directive::Text {
        content: content.to_owned(),
        span: Span { start, end },
    }
}

fn mention(name: &str, start: usize, end: usize) -> Directive {
    Directive::Mention {
        name: name.to_owned(),
        span: Span { start, end },
    }
}

fn fileref(path: &str, start: usize, end: usize) -> Directive {
    Directive::FileRef {
        path: path.to_owned(),
        span: Span { start, end },
    }
}

fn slash(name: &str, args: &[&str], start: usize, end: usize) -> Directive {
    Directive::SlashCommand {
        name: name.to_owned(),
        args: args.iter().map(|s| s.to_string()).collect(),
        span: Span { start, end },
    }
}

#[test]
fn plain_text() {
    let result = parse_input("hello world", &[]);
    assert_eq!(result.directives, vec![text("hello world", 0, 11)]);
}

#[test]
fn mention_known_agent() {
    let result = parse_input("ask @explorer to find it", &["explorer"]);
    assert_eq!(
        result.directives,
        vec![
            text("ask ", 0, 4),
            mention("explorer", 4, 13),
            text(" to find it", 13, 24),
        ]
    );
}

#[test]
fn mention_unknown_becomes_fileref() {
    let result = parse_input("look at @explorer", &[]);
    assert_eq!(
        result.directives,
        vec![text("look at ", 0, 8), fileref("explorer", 8, 17),]
    );
}

#[test]
fn file_ref_with_path() {
    let result = parse_input("check @src/main.rs", &[]);
    assert_eq!(
        result.directives,
        vec![text("check ", 0, 6), fileref("src/main.rs", 6, 18),]
    );
}

#[test]
fn slash_command_no_args() {
    let result = parse_input("/connect", &[]);
    assert_eq!(result.directives, vec![slash("connect", &[], 0, 8)]);
}

#[test]
fn slash_command_with_args() {
    let result = parse_input("/run test --verbose", &[]);
    assert_eq!(
        result.directives,
        vec![slash("run", &["test", "--verbose"], 0, 19)]
    );
}

#[test]
fn mixed_directives() {
    let result = parse_input("@explorer /search foo", &["explorer"]);
    assert_eq!(
        result.directives,
        vec![
            mention("explorer", 0, 9),
            // space between @explorer and /search is consumed as text? No — after the mention
            // ends at byte 9, the next char is a space then `/search`. The space is plain text.
            text(" ", 9, 10),
            slash("search", &["foo"], 10, 21),
        ]
    );
}

#[test]
fn escaped_at() {
    let result = parse_input(r"email is user\@host.com", &[]);
    assert_eq!(
        result.directives,
        vec![text("email is user@host.com", 0, 23)]
    );
}

#[test]
fn escaped_slash() {
    let result = parse_input(r"path is \/usr\/bin", &[]);
    assert_eq!(result.directives, vec![text("path is /usr/bin", 0, 18)]);
}

#[test]
fn slash_mid_word_not_directive() {
    let result = parse_input("http://example.com", &[]);
    assert_eq!(result.directives, vec![text("http://example.com", 0, 18)]);
}

#[test]
fn multiple_mentions() {
    let result = parse_input("@agent1 and @agent2", &["agent1", "agent2"]);
    assert_eq!(
        result.directives,
        vec![
            mention("agent1", 0, 7),
            text(" and ", 7, 12),
            mention("agent2", 12, 19),
        ]
    );
}

#[test]
fn span_offsets_correct() {
    let input = "ask @explorer to find it";
    let result = parse_input(input, &["explorer"]);

    // Find the Mention directive and verify its span indexes correctly into the input.
    let mention_dir = result
        .directives
        .iter()
        .find(|d| matches!(d, Directive::Mention { .. }));
    let Some(Directive::Mention { name, span }) = mention_dir else {
        panic!("expected a Mention directive");
    };
    assert_eq!(name, "explorer");
    // span covers `@explorer` (9 bytes), starting at byte 4.
    assert_eq!(&input[span.start..span.end], "@explorer");
}

#[test]
fn empty_input() {
    let result = parse_input("", &[]);
    assert!(result.directives.is_empty());
}

#[test]
fn at_with_dot_is_fileref() {
    // `@config.toml` contains `.` so it's always a FileRef regardless of known_agents.
    let result = parse_input("@config.toml", &["config.toml"]);
    assert_eq!(result.directives, vec![fileref("config.toml", 0, 12)]);
}

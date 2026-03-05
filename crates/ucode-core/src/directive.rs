/// Byte offset range in the original input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// A parsed directive from user input.
#[derive(Debug, Clone, PartialEq)]
pub enum Directive {
    /// `@agent-name` — spawn a named agent.
    Mention { name: String, span: Span },
    /// `/command arg1 arg2` — execute a slash command.
    SlashCommand {
        name: String,
        args: Vec<String>,
        span: Span,
    },
    /// `@path/to/file` — a file reference (not an agent mention).
    FileRef { path: String, span: Span },
    /// Plain text segment (no directive).
    Text { content: String, span: Span },
}

/// Result of parsing an input string.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedInput {
    pub directives: Vec<Directive>,
}

/// Parse user input into a sequence of directives.
///
/// Resolver order: slash command > agent mention > file reference.
///
/// Rules:
/// - `/command` at the start of a word (preceded by start-of-input or whitespace).
///   Command name is the word after `/`. Arguments are subsequent whitespace-separated tokens
///   until end of input or next directive.
/// - `@name` where `name` matches a registered agent -> Mention.
/// - `@path/to/file` or `@name` where name is NOT a registered agent -> FileRef.
/// - `\@` and `\/` are escape sequences that produce literal `@` and `/` in Text.
/// - Everything else is Text.
///
/// `known_agents` is the set of registered agent names for disambiguation.
pub fn parse_input(input: &str, known_agents: &[&str]) -> ParsedInput {
    let bytes = input.as_bytes();
    let len = input.len();

    // Pending text accumulator: (start_byte, content)
    let mut text_start: usize = 0;
    let mut text_buf = String::new();

    let mut directives: Vec<Directive> = Vec::new();
    let mut i = 0usize;

    // Flush any accumulated text into a Text directive.
    macro_rules! flush_text {
        ($end:expr) => {
            if !text_buf.is_empty() {
                directives.push(Directive::Text {
                    content: text_buf.clone(),
                    span: Span {
                        start: text_start,
                        end: $end,
                    },
                });
                text_buf.clear();
            }
        };
    }

    // Append a char to the text accumulator, recording start position.
    macro_rules! push_text {
        ($pos:expr, $ch:expr) => {
            if text_buf.is_empty() {
                text_start = $pos;
            }
            text_buf.push($ch);
        };
    }

    while i < len {
        // Check for escape sequences first.
        if bytes[i] == b'\\' && i + 1 < len && (bytes[i + 1] == b'@' || bytes[i + 1] == b'/') {
            let escaped = bytes[i + 1] as char;
            push_text!(i, escaped);
            i += 2;
            continue;
        }

        let at_word_boundary = i == 0 || bytes[i - 1].is_ascii_whitespace();

        // Slash command: `/` at a word boundary.
        if bytes[i] == b'/' && at_word_boundary {
            // Peek ahead: must have at least one non-whitespace char for the command name.
            let name_start = i + 1;
            let name_end = bytes[name_start..]
                .iter()
                .position(|&b| b.is_ascii_whitespace())
                .map(|p| name_start + p)
                .unwrap_or(len);

            if name_end > name_start {
                flush_text!(i);
                let name = input[name_start..name_end].to_owned();
                let mut pos = name_end;

                // Collect args: whitespace-separated tokens until end or next directive marker
                // at a word boundary.
                let mut args: Vec<String> = Vec::new();
                loop {
                    // Skip whitespace.
                    let ws_end = bytes[pos..]
                        .iter()
                        .position(|&b| !b.is_ascii_whitespace())
                        .map(|p| pos + p)
                        .unwrap_or(len);
                    if ws_end >= len {
                        pos = len;
                        break;
                    }
                    pos = ws_end;

                    // Stop if next token is a directive marker at a word boundary.
                    // (pos is always at a word boundary here since we just skipped whitespace)
                    if bytes[pos] == b'@' || bytes[pos] == b'/' {
                        break;
                    }
                    // Also stop on escape sequences that produce directive chars — but those
                    // are consumed as text, so we only stop on bare markers.

                    // Read the token.
                    let tok_end = bytes[pos..]
                        .iter()
                        .position(|&b| b.is_ascii_whitespace())
                        .map(|p| pos + p)
                        .unwrap_or(len);
                    args.push(input[pos..tok_end].to_owned());
                    pos = tok_end;
                }

                let span = Span { start: i, end: pos };
                directives.push(Directive::SlashCommand { name, args, span });
                i = pos;
                continue;
            }
            // No name after `/` — treat as plain text.
            push_text!(i, '/');
            i += 1;
            continue;
        }

        // `@` at a word boundary.
        if bytes[i] == b'@' && at_word_boundary {
            let name_start = i + 1;
            // Name ends at whitespace, end of input, or another bare `@`/`/` that is NOT
            // part of the path (we allow `/` and `.` inside the name for paths).
            // Actually for paths we want to allow `/` and `.` — so we read until whitespace or
            // another `@`.
            let name_end = bytes[name_start..]
                .iter()
                .position(|&b| b.is_ascii_whitespace() || b == b'@')
                .map(|p| name_start + p)
                .unwrap_or(len);

            if name_end > name_start {
                flush_text!(i);
                let token = &input[name_start..name_end];
                let span = Span {
                    start: i,
                    end: name_end,
                };

                // Classify: path-like (contains `/` or `.`) -> FileRef; else check known_agents.
                if token.contains('/') || token.contains('.') {
                    directives.push(Directive::FileRef {
                        path: token.to_owned(),
                        span,
                    });
                } else if known_agents.contains(&token) {
                    directives.push(Directive::Mention {
                        name: token.to_owned(),
                        span,
                    });
                } else {
                    directives.push(Directive::FileRef {
                        path: token.to_owned(),
                        span,
                    });
                }
                i = name_end;
                continue;
            }
            // Bare `@` with nothing after it — plain text.
            push_text!(i, '@');
            i += 1;
            continue;
        }

        // Plain character.
        push_text!(i, bytes[i] as char);
        i += 1;
    }

    flush_text!(len);

    // Merge adjacent Text segments (can arise from escape sequences interleaved with text).
    // The current design accumulates into a single buffer so merging is already done, but
    // we do a final pass to be safe and to handle the edge case where flush happened mid-text.
    let merged = merge_adjacent_text(directives);

    ParsedInput { directives: merged }
}

fn merge_adjacent_text(directives: Vec<Directive>) -> Vec<Directive> {
    let mut out: Vec<Directive> = Vec::with_capacity(directives.len());
    for d in directives {
        match d {
            Directive::Text { ref content, span } => {
                if content.is_empty() {
                    continue;
                }
                if let Some(Directive::Text {
                    content: prev,
                    span: prev_span,
                }) = out.last_mut()
                {
                    prev.push_str(content);
                    prev_span.end = span.end;
                } else {
                    out.push(d);
                }
            }
            other => out.push(other),
        }
    }
    out
}

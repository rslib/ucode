use std::sync::LazyLock;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color as SyntectColor, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings,
};
use syntect::parsing::SyntaxSet;
use unicode_width::UnicodeWidthStr;

use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// Syntect globals — loaded once, reused for every render call.
// ---------------------------------------------------------------------------

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

// ---------------------------------------------------------------------------
// Syntect theme builder
// ---------------------------------------------------------------------------

fn theme_item(scope_str: &str, rgb: &ucode_themes::Rgb) -> ThemeItem {
    ThemeItem {
        scope: scope_str.parse::<ScopeSelectors>().unwrap_or_default(),
        style: StyleModifier {
            foreground: Some(SyntectColor {
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
                a: 0xff,
            }),
            background: None,
            font_style: None,
        },
    }
}

fn build_syntect_theme(
    syntax: &ucode_themes::SyntaxColors,
    bg: &ucode_themes::Rgb,
    fg: &ucode_themes::Rgb,
) -> Theme {
    let to_sc = |rgb: &ucode_themes::Rgb| SyntectColor {
        r: rgb.r,
        g: rgb.g,
        b: rgb.b,
        a: 0xff,
    };

    let settings = ThemeSettings {
        foreground: Some(to_sc(fg)),
        background: Some(to_sc(bg)),
        ..Default::default()
    };

    let scopes = vec![
        theme_item("keyword", &syntax.keyword),
        theme_item("storage.type, storage.modifier", &syntax.keyword),
        theme_item("string", &syntax.string),
        theme_item("comment", &syntax.comment),
        theme_item(
            "entity.name.type, support.type, support.class",
            &syntax.type_name,
        ),
        theme_item("entity.name.function, support.function", &syntax.function),
        theme_item("constant.numeric", &syntax.number),
        theme_item("keyword.operator, punctuation.accessor", &syntax.operator),
        theme_item("variable, variable.parameter", &syntax.variable),
        theme_item("constant.language, constant.other", &syntax.constant),
        theme_item(
            "entity.other.attribute-name, meta.annotation",
            &syntax.attribute,
        ),
        theme_item("entity.name.tag", &syntax.tag),
        theme_item(
            "punctuation.definition, punctuation.section, punctuation.separator",
            &syntax.punctuation,
        ),
    ];

    Theme {
        name: None,
        author: None,
        settings,
        scopes,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render markdown text into styled ratatui Lines.
///
/// Each Line is a single terminal row. The caller is responsible for
/// adding indentation (2-space prefix) — this function produces raw
/// content lines without indentation.
///
/// `width` is the available content width (excluding indent).
/// If parsing fails or text has no markdown, returns plain text lines.
pub fn render_markdown<'a>(text: &str, theme: &'a UcodeTheme, width: u16) -> Vec<Line<'a>> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(text, opts);
    let events: Vec<Event<'_>> = parser.collect();

    if events.is_empty() {
        return wrap_plain(text, theme, width);
    }

    let mut ctx = RenderCtx::new(theme, width);
    ctx.process(&events);
    ctx.finish()
}

/// Estimate how many terminal rows markdown text will occupy at `width`.
pub fn markdown_height(text: &str, width: u16) -> usize {
    let theme = UcodeTheme::default();
    render_markdown(text, &theme, width).len()
}

// ---------------------------------------------------------------------------
// Internal: inline style tracking
// ---------------------------------------------------------------------------

/// Accumulated inline modifier flags (not color — color comes from context).
#[derive(Clone, Copy, Default)]
struct InlineMods {
    bold: bool,
    italic: bool,
    strikethrough: bool,
}

impl InlineMods {
    fn apply(self, base: Style) -> Style {
        let mut s = base;
        if self.bold {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.italic {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if self.strikethrough {
            s = s.add_modifier(Modifier::CROSSED_OUT);
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Internal: table accumulation
// ---------------------------------------------------------------------------

struct TableCtx {
    /// All rows; first row is the header.
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
    in_head: bool,
}

impl TableCtx {
    fn new() -> Self {
        Self {
            rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: String::new(),
            in_head: false,
        }
    }

    fn finish_cell(&mut self) {
        self.current_row.push(self.current_cell.trim().to_owned());
        self.current_cell.clear();
    }

    fn finish_row(&mut self) {
        if !self.current_row.is_empty() {
            self.rows.push(std::mem::take(&mut self.current_row));
        }
    }
}

// ---------------------------------------------------------------------------
// Internal: list nesting
// ---------------------------------------------------------------------------

struct ListLevel {
    ordered: bool,
    next_num: u64,
}

impl ListLevel {
    fn unordered() -> Self {
        Self {
            ordered: false,
            next_num: 1,
        }
    }

    fn ordered(start: u64) -> Self {
        Self {
            ordered: true,
            next_num: start,
        }
    }

    fn take_num(&mut self) -> u64 {
        let n = self.next_num;
        self.next_num += 1;
        n
    }
}

// ---------------------------------------------------------------------------
// Internal: block context
// ---------------------------------------------------------------------------

/// What kind of block we are currently inside.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockCtx {
    None,
    Paragraph,
    Heading(HeadingLevel),
    CodeBlock,
    ListItem,
    Table,
}

// ---------------------------------------------------------------------------
// Render context
// ---------------------------------------------------------------------------

struct RenderCtx<'a> {
    theme: &'a UcodeTheme,
    width: u16,

    lines: Vec<Line<'a>>,

    /// Spans for the current in-progress line.
    current_spans: Vec<Span<'a>>,

    block_ctx: BlockCtx,

    /// Inline modifier stack (push on Start, pop on End).
    mod_stack: Vec<InlineMods>,
    /// Current combined inline modifiers.
    mods: InlineMods,

    /// Heading style override (set while inside a heading).
    heading_style: Option<Style>,

    /// Link URL pending (set on Tag::Link start, consumed on TagEnd::Link).
    pending_link_url: Option<String>,

    /// List nesting.
    list_stack: Vec<ListLevel>,
    /// Text accumulated for the current list item.
    item_text: String,

    /// Table state.
    table: Option<TableCtx>,

    /// Language token for the current fenced code block (e.g. "rust", "python").
    /// None when not inside a code block or when no language was specified.
    code_lang: Option<String>,
}

impl<'a> RenderCtx<'a> {
    fn new(theme: &'a UcodeTheme, width: u16) -> Self {
        Self {
            theme,
            width,
            lines: Vec::new(),
            current_spans: Vec::new(),
            block_ctx: BlockCtx::None,
            mod_stack: Vec::new(),
            mods: InlineMods::default(),
            heading_style: None,
            pending_link_url: None,
            list_stack: Vec::new(),
            item_text: String::new(),
            table: None,
            code_lang: None,
        }
    }

    // -----------------------------------------------------------------------
    // Line helpers
    // -----------------------------------------------------------------------

    fn flush_line(&mut self) {
        let spans = std::mem::take(&mut self.current_spans);
        self.lines.push(Line::from(spans));
    }

    fn push_blank(&mut self) {
        self.lines.push(Line::from(""));
    }

    fn push_line(&mut self, line: Line<'a>) {
        self.lines.push(line);
    }

    // -----------------------------------------------------------------------
    // Span helpers
    // -----------------------------------------------------------------------

    /// Compute the style for normal inline text in the current context.
    fn inline_text_style(&self) -> Style {
        let base = if let Some(hs) = self.heading_style {
            hs
        } else {
            self.theme.text_style()
        };
        self.mods.apply(base)
    }

    fn push_span(&mut self, text: String, style: Style) {
        self.current_spans.push(Span::styled(text, style));
    }

    // -----------------------------------------------------------------------
    // Event dispatch
    // -----------------------------------------------------------------------

    fn process(&mut self, events: &[Event<'_>]) {
        for event in events {
            match event {
                Event::Start(tag) => self.on_start(tag),
                Event::End(tag) => self.on_end(tag),
                Event::Text(t) => self.on_text(t.as_ref()),
                Event::Code(c) => self.on_inline_code(c.as_ref()),
                Event::SoftBreak => self.on_soft_break(),
                Event::HardBreak => self.on_hard_break(),
                Event::Rule => self.on_rule(),
                _ => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Start events
    // -----------------------------------------------------------------------

    fn on_start(&mut self, tag: &Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.block_ctx = BlockCtx::Paragraph;
            }

            Tag::Heading { level, .. } => {
                self.block_ctx = BlockCtx::Heading(*level);
                let style = heading_style(*level, self.theme);
                self.heading_style = Some(style);
                // Push heading prefix.
                let prefix = heading_prefix(*level);
                self.push_span(prefix.to_owned(), style);
            }

            Tag::CodeBlock(kind) => {
                self.block_ctx = BlockCtx::CodeBlock;
                let lang = match kind {
                    CodeBlockKind::Fenced(l) if !l.is_empty() => l.as_ref().to_owned(),
                    _ => String::new(),
                };
                let label = format!("  {}", if lang.is_empty() { "code" } else { &lang });
                self.push_line(Line::from(Span::styled(label, self.theme.dim_style())));
                self.code_lang = if lang.is_empty() { None } else { Some(lang) };
            }

            Tag::List(start) => {
                let level = match start {
                    Some(n) => ListLevel::ordered(*n),
                    None => ListLevel::unordered(),
                };
                self.list_stack.push(level);
            }

            Tag::Item => {
                self.block_ctx = BlockCtx::ListItem;
                self.item_text.clear();
            }

            Tag::Strong => {
                self.mod_stack.push(self.mods);
                self.mods.bold = true;
            }

            Tag::Emphasis => {
                self.mod_stack.push(self.mods);
                self.mods.italic = true;
            }

            Tag::Strikethrough => {
                self.mod_stack.push(self.mods);
                self.mods.strikethrough = true;
            }

            Tag::Link { dest_url, .. } => {
                self.pending_link_url = Some(dest_url.as_ref().to_owned());
            }

            Tag::Table(_) => {
                self.block_ctx = BlockCtx::Table;
                self.table = Some(TableCtx::new());
            }

            Tag::TableHead => {
                if let Some(t) = &mut self.table {
                    t.in_head = true;
                }
            }

            // TableRow, TableCell, BlockQuote, Image, etc. — no special action.
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // End events
    // -----------------------------------------------------------------------

    fn on_end(&mut self, tag: &TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.block_ctx = BlockCtx::None;
                let spans = std::mem::take(&mut self.current_spans);
                if !spans.is_empty() {
                    for line in wrap_spans(spans, self.width) {
                        self.push_line(line);
                    }
                }
                self.push_blank();
            }

            TagEnd::Heading(_) => {
                self.heading_style = None;
                self.block_ctx = BlockCtx::None;
                self.flush_line();
                self.push_blank();
            }

            TagEnd::CodeBlock => {
                self.block_ctx = BlockCtx::None;
                self.code_lang = None;
                self.push_blank();
            }

            TagEnd::List(_) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.push_blank();
                }
            }

            TagEnd::Item => {
                self.block_ctx = BlockCtx::None;
                self.emit_list_item();
            }

            TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                if let Some(prev) = self.mod_stack.pop() {
                    self.mods = prev;
                }
            }

            TagEnd::Link => {
                if let Some(url) = self.pending_link_url.take() {
                    let dim = self.theme.dim_style();
                    self.current_spans
                        .push(Span::styled(format!(" ({url})"), dim));
                }
            }

            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    t.finish_row();
                    t.in_head = false;
                }
            }

            TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    t.finish_row();
                }
            }

            TagEnd::TableCell => {
                if let Some(t) = &mut self.table {
                    t.finish_cell();
                }
            }

            TagEnd::Table => {
                self.block_ctx = BlockCtx::None;
                if let Some(table) = self.table.take() {
                    for line in render_table(table, self.theme, self.width) {
                        self.push_line(line);
                    }
                }
                self.push_blank();
            }

            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Leaf events
    // -----------------------------------------------------------------------

    fn on_text(&mut self, text: &str) {
        match self.block_ctx {
            BlockCtx::CodeBlock => {
                // pulldown-cmark delivers the entire code block body as a
                // single Text event with embedded newlines. Split and emit
                // one terminal line per source line.
                let max = self.width as usize;
                let fallback_style = Style::new().fg(self.theme.text).bg(self.theme.surface);
                let bg = self.theme.surface;
                // Strip a single trailing newline that pulldown-cmark appends.
                let body = text.strip_suffix('\n').unwrap_or(text);

                // Attempt syntect highlighting when a language token is present.
                let syntax = self
                    .code_lang
                    .as_deref()
                    .and_then(|lang| SYNTAX_SET.find_syntax_by_token(lang));

                if let Some(syntax) = syntax {
                    let hl_theme = build_syntect_theme(
                        &self.theme.def.syntax,
                        &self.theme.def.surface,
                        &self.theme.def.text,
                    );
                    let mut hl = HighlightLines::new(syntax, &hl_theme);
                    for src_line in body.split('\n') {
                        // syntect expects a trailing newline per line.
                        let line_nl = format!("{src_line}\n");
                        let tokens = hl.highlight_line(&line_nl, &SYNTAX_SET);
                        let spans = match tokens {
                            Ok(ts) => {
                                let mut spans: Vec<Span<'a>> = Vec::new();
                                // 2-space indent prefix with surface bg.
                                spans.push(Span::styled("  ", fallback_style));
                                let mut col = 2usize;
                                'tokens: for (hl_style, token) in ts {
                                    // Strip the trailing newline syntect re-appends.
                                    let token = token.trim_end_matches('\n');
                                    if token.is_empty() {
                                        continue;
                                    }
                                    let fg = hl_style.foreground;
                                    let span_style =
                                        Style::new().fg(Color::Rgb(fg.r, fg.g, fg.b)).bg(bg);
                                    // Truncate to max width.
                                    let remaining = max.saturating_sub(col);
                                    if remaining == 0 {
                                        break 'tokens;
                                    }
                                    let text_w = token.width();
                                    let text_out = if col + text_w > max {
                                        token.chars().take(remaining).collect::<String>()
                                    } else {
                                        token.to_owned()
                                    };
                                    col += text_out.width();
                                    spans.push(Span::styled(text_out, span_style));
                                }
                                spans
                            }
                            // Highlighting failed — fall back to plain for this line.
                            Err(_) => {
                                let content = format!("  {src_line}");
                                let display: String = if content.width() > max {
                                    content.chars().take(max).collect()
                                } else {
                                    content
                                };
                                vec![Span::styled(display, fallback_style)]
                            }
                        };
                        self.push_line(Line::from(spans));
                    }
                    return;
                }

                // No syntax found or no theme — plain rendering.
                for src_line in body.split('\n') {
                    let content = format!("  {src_line}");
                    let display: String = if content.width() > max {
                        content.chars().take(max).collect()
                    } else {
                        content
                    };
                    self.push_line(Line::from(Span::styled(display, fallback_style)));
                }
            }

            BlockCtx::Table => {
                if let Some(t) = &mut self.table {
                    t.current_cell.push_str(text);
                }
            }

            BlockCtx::ListItem => {
                self.item_text.push_str(text);
            }

            _ => {
                // Paragraph, Heading, or bare text.
                let style = self.inline_text_style();
                // For links, apply accent + underline to the link text.
                let style = if self.pending_link_url.is_some() {
                    style
                        .fg(self.theme.accent)
                        .add_modifier(Modifier::UNDERLINED)
                } else {
                    style
                };
                self.push_span(text.to_owned(), style);
            }
        }
    }

    fn on_inline_code(&mut self, code: &str) {
        match self.block_ctx {
            BlockCtx::Table => {
                if let Some(t) = &mut self.table {
                    t.current_cell.push_str(code);
                }
            }
            BlockCtx::ListItem => {
                self.item_text.push_str(code);
            }
            _ => {
                let style = Style::new().fg(self.theme.accent).bg(self.theme.surface);
                self.push_span(code.to_owned(), style);
            }
        }
    }

    fn on_soft_break(&mut self) {
        match self.block_ctx {
            BlockCtx::ListItem => self.item_text.push(' '),
            BlockCtx::Table => {
                if let Some(t) = &mut self.table {
                    t.current_cell.push(' ');
                }
            }
            _ => {
                // In paragraphs/headings: soft break is a space.
                let style = self.inline_text_style();
                self.push_span(" ".to_owned(), style);
            }
        }
    }

    fn on_hard_break(&mut self) {
        match self.block_ctx {
            BlockCtx::ListItem => self.item_text.push('\n'),
            _ => {
                let spans = std::mem::take(&mut self.current_spans);
                if !spans.is_empty() {
                    for line in wrap_spans(spans, self.width) {
                        self.push_line(line);
                    }
                } else {
                    self.push_blank();
                }
            }
        }
    }

    fn on_rule(&mut self) {
        let w = (self.width as usize).min(80);
        let rule = "─".repeat(w);
        self.push_line(Line::from(Span::styled(rule, self.theme.muted_style())));
    }

    // -----------------------------------------------------------------------
    // List item emission
    // -----------------------------------------------------------------------

    fn emit_list_item(&mut self) {
        // depth 0 = top-level list, depth 1 = one nested, etc.
        let depth = self.list_stack.len().saturating_sub(1);
        let outer_indent = "  ".repeat(depth);

        let prefix = if let Some(level) = self.list_stack.last_mut() {
            if level.ordered {
                let n = level.take_num();
                format!("{outer_indent}  {n}. ")
            } else {
                format!("{outer_indent}  - ")
            }
        } else {
            "  - ".to_owned()
        };

        let text = std::mem::take(&mut self.item_text);
        let prefix_w = prefix.width();
        let content_w = (self.width as usize).saturating_sub(prefix_w);
        let continuation = " ".repeat(prefix_w);

        let wrapped = wrap_text_str(&text, content_w);

        if wrapped.is_empty() {
            self.push_line(Line::from(Span::styled(prefix, self.theme.text_style())));
            return;
        }

        for (i, chunk) in wrapped.iter().enumerate() {
            let lead = if i == 0 {
                prefix.clone()
            } else {
                continuation.clone()
            };
            self.push_line(Line::from(vec![
                Span::styled(lead, self.theme.text_style()),
                Span::styled(chunk.clone(), self.theme.text_style()),
            ]));
        }
    }

    // -----------------------------------------------------------------------
    // Finalise
    // -----------------------------------------------------------------------

    fn finish(mut self) -> Vec<Line<'a>> {
        if !self.current_spans.is_empty() {
            let spans = std::mem::take(&mut self.current_spans);
            for line in wrap_spans(spans, self.width) {
                self.lines.push(line);
            }
        }
        // Strip trailing blank lines.
        while self.lines.last().is_some_and(|l| {
            l.spans.is_empty() || l.spans.iter().all(|s| s.content.trim().is_empty())
        }) {
            self.lines.pop();
        }
        self.lines
    }
}

// ---------------------------------------------------------------------------
// Heading helpers
// ---------------------------------------------------------------------------

fn heading_style(level: HeadingLevel, theme: &UcodeTheme) -> Style {
    match level {
        HeadingLevel::H1 => theme
            .accent_style()
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::UNDERLINED),
        HeadingLevel::H2 => theme.accent_style().add_modifier(Modifier::BOLD),
        _ => theme.accent_style(),
    }
}

fn heading_prefix(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "# ",
        HeadingLevel::H2 => "## ",
        HeadingLevel::H3 => "### ",
        HeadingLevel::H4 => "#### ",
        HeadingLevel::H5 => "##### ",
        HeadingLevel::H6 => "###### ",
    }
}

// ---------------------------------------------------------------------------
// Table rendering
// ---------------------------------------------------------------------------

fn render_table<'a>(table: TableCtx, theme: &'a UcodeTheme, width: u16) -> Vec<Line<'a>> {
    if table.rows.is_empty() {
        return Vec::new();
    }

    let col_count = table.rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if col_count == 0 {
        return Vec::new();
    }

    // Measure column widths.
    let mut col_widths: Vec<usize> = vec![0; col_count];
    for row in &table.rows {
        for (ci, cell) in row.iter().enumerate() {
            if ci < col_count {
                col_widths[ci] = col_widths[ci].max(cell.width());
            }
        }
    }

    let pipe_style = theme.muted_style();
    let text_style = theme.text_style();
    let bold_style = text_style.add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line<'a>> = Vec::new();

    let render_row = |row: &Vec<String>, row_style: Style, col_widths: &[usize]| -> Line<'a> {
        let mut spans: Vec<Span<'a>> = Vec::new();
        spans.push(Span::styled("| ".to_owned(), pipe_style));
        for (ci, cell) in row.iter().enumerate() {
            let w = col_widths.get(ci).copied().unwrap_or(0);
            let padded = format!("{:<width$}", cell, width = w);
            spans.push(Span::styled(padded, row_style));
            spans.push(Span::styled(" | ".to_owned(), pipe_style));
        }
        // Fill missing columns.
        for ci in row.len()..col_count {
            let w = col_widths.get(ci).copied().unwrap_or(0);
            let padded = " ".repeat(w);
            spans.push(Span::styled(padded, row_style));
            spans.push(Span::styled(" | ".to_owned(), pipe_style));
        }
        Line::from(spans)
    };

    let _ = width; // width used for future truncation; table renders as-is

    for (ri, row) in table.rows.iter().enumerate() {
        if ri == 0 {
            // Header row.
            lines.push(render_row(row, bold_style, &col_widths));
            // Separator.
            let mut sep_spans: Vec<Span<'a>> = Vec::new();
            sep_spans.push(Span::styled("| ".to_owned(), pipe_style));
            for w in &col_widths {
                let dashes = "-".repeat(*w);
                sep_spans.push(Span::styled(dashes, pipe_style));
                sep_spans.push(Span::styled(" | ".to_owned(), pipe_style));
            }
            lines.push(Line::from(sep_spans));
        } else {
            lines.push(render_row(row, text_style, &col_widths));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Word-wrap helpers
// ---------------------------------------------------------------------------

/// Wrap a flat string into lines of at most `max_width` terminal columns.
fn wrap_text_str(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_w: usize = 0;

    for word in text.split_whitespace() {
        let word_w = word.width();
        if current.is_empty() {
            current.push_str(word);
            current_w = word_w;
        } else if current_w + 1 + word_w <= max_width {
            current.push(' ');
            current.push_str(word);
            current_w += 1 + word_w;
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
            current_w = word_w;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Word-wrap a sequence of styled spans into multiple Lines.
///
/// We concatenate all span text, split on whitespace, and re-assign each
/// word to the span whose original range it came from. This preserves
/// per-word styling while reflowing at `width`.
fn wrap_spans<'a>(spans: Vec<Span<'a>>, width: u16) -> Vec<Line<'a>> {
    if width == 0 {
        return vec![Line::from(spans)];
    }
    let max = width as usize;

    // Flatten spans into (word, style) pairs.
    let mut words: Vec<(String, Style)> = Vec::new();
    for span in spans {
        let style = span.style;
        for word in span.content.split_whitespace() {
            words.push((word.to_owned(), style));
        }
        // Preserve trailing space as a hint for inter-span spacing — handled
        // implicitly by the split_whitespace join below.
    }

    if words.is_empty() {
        return Vec::new();
    }

    let mut lines: Vec<Line<'a>> = Vec::new();
    let mut row: Vec<Span<'a>> = Vec::new();
    let mut row_w: usize = 0;

    for (word, style) in words {
        let word_w = word.width();
        if row.is_empty() {
            row.push(Span::styled(word, style));
            row_w = word_w;
        } else if row_w + 1 + word_w <= max {
            // Append space + word. Try to merge with last span if same style.
            if row.last().is_some_and(|s| s.style == style) {
                if let Some(last) = row.last_mut() {
                    let mut content = last.content.to_string();
                    content.push(' ');
                    content.push_str(&word);
                    *last = Span::styled(content, style);
                }
            } else {
                row.push(Span::styled(format!(" {word}"), style));
            }
            row_w += 1 + word_w;
        } else {
            lines.push(Line::from(std::mem::take(&mut row)));
            row.push(Span::styled(word, style));
            row_w = word_w;
        }
    }
    if !row.is_empty() {
        lines.push(Line::from(row));
    }
    lines
}

/// Wrap plain text (no markdown) into styled lines.
fn wrap_plain<'a>(text: &str, theme: &'a UcodeTheme, width: u16) -> Vec<Line<'a>> {
    let max = if width == 0 { 80 } else { width as usize };
    let mut lines: Vec<Line<'a>> = Vec::new();
    for src_line in text.lines() {
        let wrapped = wrap_text_str(src_line, max);
        if wrapped.is_empty() {
            lines.push(Line::from(""));
        } else {
            for chunk in wrapped {
                lines.push(Line::from(Span::styled(chunk, theme.text_style())));
            }
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> UcodeTheme {
        UcodeTheme::default()
    }

    /// Collect all text content from a slice of Lines.
    fn lines_text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Collect all spans from all lines into a flat vec.
    fn all_spans(lines: &[Line<'_>]) -> Vec<(String, Style)> {
        lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| (s.content.to_string(), s.style)))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Inline styles
    // -----------------------------------------------------------------------

    #[test]
    fn plain_text_single_span_text_style() {
        let t = theme();
        let lines = render_markdown("hello world", &t, 80);
        assert!(!lines.is_empty());
        let text = lines_text(&lines);
        assert!(text.contains("hello world"), "got: {text:?}");
        // All spans should use text color.
        for (_, style) in all_spans(&lines) {
            assert_eq!(style.fg, Some(t.text), "expected text color, got {style:?}");
        }
    }

    #[test]
    fn bold_text_gets_bold_modifier() {
        let t = theme();
        let lines = render_markdown("**bold**", &t, 80);
        let spans = all_spans(&lines);
        let bold_span = spans.iter().find(|(content, _)| content.contains("bold"));
        assert!(bold_span.is_some(), "no span containing 'bold': {spans:?}");
        let (_, style) = bold_span.unwrap();
        assert!(
            style.add_modifier.contains(Modifier::BOLD),
            "expected BOLD modifier, got {style:?}"
        );
    }

    #[test]
    fn italic_text_gets_italic_modifier() {
        let t = theme();
        let lines = render_markdown("*italic*", &t, 80);
        let spans = all_spans(&lines);
        let span = spans.iter().find(|(c, _)| c.contains("italic"));
        assert!(span.is_some(), "no span with 'italic': {spans:?}");
        let (_, style) = span.unwrap();
        assert!(
            style.add_modifier.contains(Modifier::ITALIC),
            "expected ITALIC, got {style:?}"
        );
    }

    #[test]
    fn strikethrough_gets_crossed_out_modifier() {
        let t = theme();
        let lines = render_markdown("~~strike~~", &t, 80);
        let spans = all_spans(&lines);
        let span = spans.iter().find(|(c, _)| c.contains("strike"));
        assert!(span.is_some(), "no span with 'strike': {spans:?}");
        let (_, style) = span.unwrap();
        assert!(
            style.add_modifier.contains(Modifier::CROSSED_OUT),
            "expected CROSSED_OUT, got {style:?}"
        );
    }

    #[test]
    fn inline_code_gets_accent_fg_and_surface_bg() {
        let t = theme();
        let lines = render_markdown("`code`", &t, 80);
        let spans = all_spans(&lines);
        let span = spans.iter().find(|(c, _)| c.contains("code"));
        assert!(span.is_some(), "no span with 'code': {spans:?}");
        let (_, style) = span.unwrap();
        assert_eq!(
            style.fg,
            Some(t.accent),
            "expected accent fg, got {style:?}"
        );
        assert_eq!(
            style.bg,
            Some(t.surface),
            "expected surface bg, got {style:?}"
        );
    }

    #[test]
    fn nested_bold_italic() {
        let t = theme();
        let lines = render_markdown("***bold italic***", &t, 80);
        let spans = all_spans(&lines);
        let span = spans.iter().find(|(c, _)| c.contains("bold italic"));
        assert!(span.is_some(), "no span with 'bold italic': {spans:?}");
        let (_, style) = span.unwrap();
        assert!(
            style.add_modifier.contains(Modifier::BOLD),
            "expected BOLD in nested bold+italic, got {style:?}"
        );
        assert!(
            style.add_modifier.contains(Modifier::ITALIC),
            "expected ITALIC in nested bold+italic, got {style:?}"
        );
    }

    #[test]
    fn mixed_normal_bold_normal() {
        let t = theme();
        let lines = render_markdown("normal **bold** normal", &t, 80);
        let text = lines_text(&lines);
        assert!(text.contains("normal"), "got: {text:?}");
        assert!(text.contains("bold"), "got: {text:?}");
        let spans = all_spans(&lines);
        let bold_span = spans
            .iter()
            .find(|(c, s)| c.contains("bold") && s.add_modifier.contains(Modifier::BOLD));
        assert!(bold_span.is_some(), "no bold span found: {spans:?}");
    }

    #[test]
    fn multiple_inline_codes_in_one_line() {
        let t = theme();
        let lines = render_markdown("`foo` and `bar`", &t, 80);
        let spans = all_spans(&lines);
        let code_spans: Vec<_> = spans
            .iter()
            .filter(|(_, s)| s.fg == Some(t.accent) && s.bg == Some(t.surface))
            .collect();
        assert_eq!(
            code_spans.len(),
            2,
            "expected 2 code spans, got: {code_spans:?}"
        );
    }

    #[test]
    fn empty_input_returns_empty_vec() {
        let t = theme();
        let lines = render_markdown("", &t, 80);
        assert!(lines.is_empty(), "expected empty, got {lines:?}");
    }

    #[test]
    fn whitespace_only_returns_empty_or_blank() {
        let t = theme();
        let lines = render_markdown("   ", &t, 80);
        // Either empty or all-blank lines.
        for line in &lines {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(text.trim().is_empty(), "expected blank, got {text:?}");
        }
    }

    #[test]
    fn link_renders_text_and_url() {
        let t = theme();
        let lines = render_markdown("[click here](https://example.com)", &t, 80);
        let text = lines_text(&lines);
        assert!(text.contains("click here"), "got: {text:?}");
        assert!(text.contains("https://example.com"), "got: {text:?}");
    }

    #[test]
    fn link_text_has_accent_underline() {
        let t = theme();
        let lines = render_markdown("[click](https://example.com)", &t, 80);
        let spans = all_spans(&lines);
        let link_span = spans.iter().find(|(c, _)| c.contains("click"));
        assert!(link_span.is_some(), "no span with 'click': {spans:?}");
        let (_, style) = link_span.unwrap();
        assert_eq!(style.fg, Some(t.accent), "expected accent fg for link text");
        assert!(
            style.add_modifier.contains(Modifier::UNDERLINED),
            "expected UNDERLINED for link text, got {style:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Code blocks
    // -----------------------------------------------------------------------

    #[test]
    fn code_block_with_language_label() {
        let t = theme();
        let md = "```rust\nfn main() {}\n```";
        let lines = render_markdown(md, &t, 80);
        let text = lines_text(&lines);
        assert!(
            text.contains("rust"),
            "expected language label, got: {text:?}"
        );
        assert!(
            text.contains("fn main()"),
            "expected code content, got: {text:?}"
        );
    }

    #[test]
    fn code_block_without_language_uses_code_label() {
        let t = theme();
        let md = "```\nhello\n```";
        let lines = render_markdown(md, &t, 80);
        let text = lines_text(&lines);
        assert!(
            text.contains("code"),
            "expected 'code' label, got: {text:?}"
        );
    }

    #[test]
    fn code_block_label_uses_dim_style() {
        let t = theme();
        let md = "```rust\nfn main() {}\n```";
        let lines = render_markdown(md, &t, 80);
        // First line should be the label with dim style.
        let first = &lines[0];
        let label_span = first.spans.iter().find(|s| s.content.contains("rust"));
        assert!(label_span.is_some(), "no label span: {first:?}");
        let style = label_span.unwrap().style;
        assert_eq!(
            style.fg,
            Some(t.text_dim),
            "expected dim fg for label, got {style:?}"
        );
    }

    #[test]
    fn code_block_content_has_surface_bg() {
        let t = theme();
        let md = "```\nhello\n```";
        let lines = render_markdown(md, &t, 80);
        let spans = all_spans(&lines);
        let code_span = spans.iter().find(|(c, _)| c.contains("hello"));
        assert!(code_span.is_some(), "no span with 'hello': {spans:?}");
        let (_, style) = code_span.unwrap();
        assert_eq!(
            style.bg,
            Some(t.surface),
            "expected surface bg, got {style:?}"
        );
    }

    #[test]
    fn code_block_not_word_wrapped() {
        let t = theme();
        // A long single line — should not be split.
        let long_line = "a".repeat(30);
        let md = format!("```\n{long_line}\n```");
        let lines = render_markdown(&md, &t, 20);
        // The code content line should contain the long line (possibly truncated, but not split).
        let code_lines: Vec<_> = lines
            .iter()
            .filter(|l| {
                l.spans
                    .iter()
                    .any(|s| s.style.bg == Some(t.surface) && s.content.contains('a'))
            })
            .collect();
        assert_eq!(
            code_lines.len(),
            1,
            "code should be on one line, got: {code_lines:?}"
        );
    }

    #[test]
    fn multi_line_code_block() {
        let t = theme();
        let md = "```\nline1\nline2\nline3\n```";
        let lines = render_markdown(md, &t, 80);
        let text = lines_text(&lines);
        assert!(text.contains("line1"), "got: {text:?}");
        assert!(text.contains("line2"), "got: {text:?}");
        assert!(text.contains("line3"), "got: {text:?}");
    }

    // -----------------------------------------------------------------------
    // Headers
    // -----------------------------------------------------------------------

    #[test]
    fn h1_gets_accent_bold_underline() {
        let t = theme();
        let lines = render_markdown("# Title", &t, 80);
        let spans = all_spans(&lines);
        let title_span = spans.iter().find(|(c, _)| c.contains("Title"));
        assert!(title_span.is_some(), "no span with 'Title': {spans:?}");
        let (_, style) = title_span.unwrap();
        assert_eq!(style.fg, Some(t.accent), "expected accent fg for H1");
        assert!(
            style.add_modifier.contains(Modifier::BOLD),
            "expected BOLD for H1, got {style:?}"
        );
        assert!(
            style.add_modifier.contains(Modifier::UNDERLINED),
            "expected UNDERLINED for H1, got {style:?}"
        );
    }

    #[test]
    fn h2_gets_accent_bold() {
        let t = theme();
        let lines = render_markdown("## Subtitle", &t, 80);
        let spans = all_spans(&lines);
        let span = spans.iter().find(|(c, _)| c.contains("Subtitle"));
        assert!(span.is_some(), "no span with 'Subtitle': {spans:?}");
        let (_, style) = span.unwrap();
        assert_eq!(style.fg, Some(t.accent), "expected accent fg for H2");
        assert!(
            style.add_modifier.contains(Modifier::BOLD),
            "expected BOLD for H2, got {style:?}"
        );
        assert!(
            !style.add_modifier.contains(Modifier::UNDERLINED),
            "H2 should not be underlined, got {style:?}"
        );
    }

    #[test]
    fn h3_gets_accent_only() {
        let t = theme();
        let lines = render_markdown("### Section", &t, 80);
        let spans = all_spans(&lines);
        let span = spans.iter().find(|(c, _)| c.contains("Section"));
        assert!(span.is_some(), "no span with 'Section': {spans:?}");
        let (_, style) = span.unwrap();
        assert_eq!(style.fg, Some(t.accent), "expected accent fg for H3");
    }

    #[test]
    fn h4_h5_h6_get_accent() {
        let t = theme();
        for (level, text) in [("####", "Four"), ("#####", "Five"), ("######", "Six")] {
            let md = format!("{level} {text}");
            let lines = render_markdown(&md, &t, 80);
            let spans = all_spans(&lines);
            let span = spans.iter().find(|(c, _)| c.contains(text));
            assert!(span.is_some(), "no span with '{text}': {spans:?}");
            let (_, style) = span.unwrap();
            assert_eq!(
                style.fg,
                Some(t.accent),
                "expected accent fg for {level}, got {style:?}"
            );
        }
    }

    #[test]
    fn header_followed_by_paragraph_has_blank_line() {
        let t = theme();
        let md = "# Title\n\nParagraph text.";
        let lines = render_markdown(md, &t, 80);
        let text_lines: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        // Find the title line index.
        let title_idx = text_lines
            .iter()
            .position(|l| l.contains("Title"))
            .expect("no title line");
        // There should be a blank line after the title.
        assert!(title_idx + 1 < text_lines.len(), "no line after title");
        let after_title = &text_lines[title_idx + 1];
        assert!(
            after_title.trim().is_empty(),
            "expected blank line after header, got {after_title:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Lists
    // -----------------------------------------------------------------------

    #[test]
    fn bullet_list_items_get_dash_prefix() {
        let t = theme();
        let md = "- item one\n- item two";
        let lines = render_markdown(md, &t, 80);
        let text = lines_text(&lines);
        assert!(text.contains("- item one"), "got: {text:?}");
        assert!(text.contains("- item two"), "got: {text:?}");
    }

    #[test]
    fn numbered_list_items_get_number_prefix() {
        let t = theme();
        let md = "1. first\n2. second\n3. third";
        let lines = render_markdown(md, &t, 80);
        let text = lines_text(&lines);
        assert!(text.contains("1."), "expected '1.' prefix, got: {text:?}");
        assert!(text.contains("2."), "expected '2.' prefix, got: {text:?}");
        assert!(text.contains("3."), "expected '3.' prefix, got: {text:?}");
    }

    #[test]
    fn multi_item_bullet_list() {
        let t = theme();
        let md = "- alpha\n- beta\n- gamma";
        let lines = render_markdown(md, &t, 80);
        let text = lines_text(&lines);
        assert!(text.contains("alpha"), "got: {text:?}");
        assert!(text.contains("beta"), "got: {text:?}");
        assert!(text.contains("gamma"), "got: {text:?}");
    }

    #[test]
    fn long_list_item_wraps_with_indent() {
        let t = theme();
        // Item text that definitely wraps at width=20.
        let item = "word ".repeat(10).trim().to_owned();
        let md = format!("- {item}");
        let lines = render_markdown(&md, &t, 20);
        // Should produce more than one line.
        assert!(
            lines.len() > 1,
            "expected wrapping, got {} lines: {lines:?}",
            lines.len()
        );
        // Continuation lines should be indented (not start with "- ").
        for line in lines.iter().skip(1) {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if !text.trim().is_empty() {
                assert!(
                    !text.trim_start().starts_with('-'),
                    "continuation line should not start with '-', got {text:?}"
                );
            }
        }
    }

    #[test]
    fn list_item_prefix_has_two_space_indent() {
        let t = theme();
        let md = "- item";
        let lines = render_markdown(md, &t, 80);
        assert!(!lines.is_empty());
        let first_line_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            first_line_text.starts_with("  - "),
            "expected '  - ' prefix, got {first_line_text:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Tables
    // -----------------------------------------------------------------------

    #[test]
    fn simple_table_renders_with_pipes() {
        let t = theme();
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let lines = render_markdown(md, &t, 80);
        let text = lines_text(&lines);
        assert!(
            text.contains('|'),
            "expected pipe separators, got: {text:?}"
        );
        assert!(text.contains('A'), "expected header 'A', got: {text:?}");
        assert!(text.contains('B'), "expected header 'B', got: {text:?}");
    }

    #[test]
    fn table_header_row_is_bold() {
        let t = theme();
        let md = "| Name | Value |\n|------|-------|\n| foo  | bar   |";
        let lines = render_markdown(md, &t, 80);
        // First data line (after label) should have bold spans for header cells.
        let spans = all_spans(&lines);
        let name_span = spans.iter().find(|(c, _)| c.contains("Name"));
        assert!(name_span.is_some(), "no span with 'Name': {spans:?}");
        let (_, style) = name_span.unwrap();
        assert!(
            style.add_modifier.contains(Modifier::BOLD),
            "expected BOLD for header cell, got {style:?}"
        );
    }

    #[test]
    fn table_separator_row_present() {
        let t = theme();
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let lines = render_markdown(md, &t, 80);
        let text = lines_text(&lines);
        assert!(
            text.contains('-'),
            "expected separator dashes, got: {text:?}"
        );
    }

    #[test]
    fn table_column_widths_accommodate_longest_cell() {
        let t = theme();
        let md = "| Short | A very long header |\n|-------|--------------------|\n| x | y |";
        let lines = render_markdown(md, &t, 120);
        let text = lines_text(&lines);
        // The header "A very long header" should appear fully.
        assert!(
            text.contains("A very long header"),
            "expected full header text, got: {text:?}"
        );
    }

    #[test]
    fn table_data_rows_not_bold() {
        let t = theme();
        let md = "| Name | Value |\n|------|-------|\n| foo  | bar   |";
        let lines = render_markdown(md, &t, 80);
        let spans = all_spans(&lines);
        let foo_span = spans.iter().find(|(c, _)| c.contains("foo"));
        assert!(foo_span.is_some(), "no span with 'foo': {spans:?}");
        let (_, style) = foo_span.unwrap();
        assert!(
            !style.add_modifier.contains(Modifier::BOLD),
            "data row should not be bold, got {style:?}"
        );
    }

    #[test]
    fn table_pipe_separators_use_muted_color() {
        let t = theme();
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let lines = render_markdown(md, &t, 80);
        let spans = all_spans(&lines);
        let pipe_span = spans.iter().find(|(c, _)| c.contains('|'));
        assert!(pipe_span.is_some(), "no pipe span found: {spans:?}");
        let (_, style) = pipe_span.unwrap();
        assert_eq!(
            style.fg,
            Some(t.muted),
            "expected muted color for pipes, got {style:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Links
    // -----------------------------------------------------------------------

    #[test]
    fn link_in_paragraph_context() {
        let t = theme();
        let md = "See [the docs](https://docs.rs) for more.";
        let lines = render_markdown(md, &t, 80);
        let text = lines_text(&lines);
        assert!(text.contains("the docs"), "got: {text:?}");
        assert!(text.contains("https://docs.rs"), "got: {text:?}");
        assert!(text.contains("more"), "got: {text:?}");
    }

    #[test]
    fn link_url_in_dim_style() {
        let t = theme();
        let md = "[text](https://example.com)";
        let lines = render_markdown(md, &t, 80);
        let spans = all_spans(&lines);
        let url_span = spans
            .iter()
            .find(|(c, _)| c.contains("https://example.com"));
        assert!(url_span.is_some(), "no url span: {spans:?}");
        let (_, style) = url_span.unwrap();
        assert_eq!(
            style.fg,
            Some(t.text_dim),
            "expected dim color for url, got {style:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Paragraphs
    // -----------------------------------------------------------------------

    #[test]
    fn single_paragraph_wraps_at_width() {
        let t = theme();
        let words = "word ".repeat(20);
        let md = words.trim();
        let lines = render_markdown(md, &t, 20);
        assert!(
            lines.len() > 1,
            "expected wrapping at width=20, got {} lines",
            lines.len()
        );
        for line in &lines {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                text.width() <= 20,
                "line exceeds width=20: {text:?} (width={})",
                text.width()
            );
        }
    }

    #[test]
    fn two_paragraphs_separated_by_blank_line() {
        let t = theme();
        let md = "First paragraph.\n\nSecond paragraph.";
        let lines = render_markdown(md, &t, 80);
        let text_lines: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let first_idx = text_lines
            .iter()
            .position(|l| l.contains("First"))
            .expect("no 'First' line");
        let second_idx = text_lines
            .iter()
            .position(|l| l.contains("Second"))
            .expect("no 'Second' line");
        // There must be at least one blank line between them.
        let between: Vec<_> = text_lines[first_idx + 1..second_idx]
            .iter()
            .filter(|l| l.trim().is_empty())
            .collect();
        assert!(
            !between.is_empty(),
            "expected blank line between paragraphs, text_lines: {text_lines:?}"
        );
    }

    #[test]
    fn paragraph_after_header() {
        let t = theme();
        let md = "# Header\n\nBody text here.";
        let lines = render_markdown(md, &t, 80);
        let text = lines_text(&lines);
        assert!(text.contains("Header"), "got: {text:?}");
        assert!(text.contains("Body text here"), "got: {text:?}");
    }

    // -----------------------------------------------------------------------
    // Height estimation
    // -----------------------------------------------------------------------

    #[test]
    fn markdown_height_matches_render_output_length() {
        let md = "# Title\n\nSome paragraph text.\n\n- item one\n- item two";
        let h = markdown_height(md, 80);
        let t = UcodeTheme::default();
        let lines = render_markdown(md, &t, 80);
        assert_eq!(h, lines.len(), "height mismatch");
    }

    #[test]
    fn code_block_height() {
        let md = "```\nline1\nline2\nline3\n```";
        let h = markdown_height(md, 80);
        // 1 label + 3 content lines = 4 minimum.
        assert!(h >= 4, "expected at least 4 lines for code block, got {h}");
    }

    #[test]
    fn table_height() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |";
        let h = markdown_height(md, 80);
        // 1 header + 1 separator + 2 data rows = 4 minimum.
        assert!(h >= 4, "expected at least 4 lines for table, got {h}");
    }

    #[test]
    fn height_zero_for_empty_input() {
        assert_eq!(markdown_height("", 80), 0);
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn horizontal_rule_renders() {
        let t = theme();
        let md = "before\n\n---\n\nafter";
        let lines = render_markdown(md, &t, 80);
        let text = lines_text(&lines);
        assert!(
            text.contains('─'),
            "expected horizontal rule, got: {text:?}"
        );
    }

    #[test]
    fn code_block_indented_two_spaces() {
        let t = theme();
        let md = "```\nhello\n```";
        let lines = render_markdown(md, &t, 80);
        let code_line = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("hello")));
        assert!(code_line.is_some(), "no code line found");
        let text: String = code_line
            .unwrap()
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            text.starts_with("  "),
            "expected 2-space indent in code block, got {text:?}"
        );
    }

    #[test]
    fn render_markdown_width_zero_does_not_panic() {
        let t = theme();
        // Should not panic even with zero width.
        let _ = render_markdown("hello world", &t, 0);
    }

    #[test]
    fn link_with_no_url_does_not_panic() {
        let t = theme();
        // Autolink or bare text — should not panic.
        let _ = render_markdown("[text]()", &t, 80);
    }
}

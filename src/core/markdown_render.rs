//! Render markdown as styled text for a terminal.
//!
//! Parsing is delegated to `pulldown-cmark`; this module owns the layout:
//! width-aware wrapping, list and blockquote prefixes, and the SGR codes.
//! There is no syntax highlighting — fenced code blocks are emitted verbatim.

use std::fmt::Write as _;

use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthStr;

const RESET: &str = "\x1b[0m";
const HEADING_COLOR: u8 = 39;
const CODE_COLOR: u8 = 214;
const LINK_COLOR: u8 = 33;

/// Render markdown for stdout, styling with ANSI only when stdout is a
/// terminal and wrapping to the terminal width.
pub fn render_for_stdout(markdown: &str) -> String {
    use std::io::IsTerminal;

    let width = terminal_size::terminal_size()
        .map(|(width, _)| usize::from(width.0))
        .unwrap_or(80);
    render(markdown, width, std::io::stdout().is_terminal())
}

/// Render markdown to a string wrapped to `width` columns, emitting ANSI
/// escape codes when `ansi` is set.
pub fn render(markdown: &str, width: usize, ansi: bool) -> String {
    let items = Layout::default().run(markdown);
    emit(&items, width.max(20), ansi)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Style {
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    color: Option<u8>,
}

impl Style {
    fn dim() -> Self {
        Self {
            dim: true,
            ..Self::default()
        }
    }
}

#[derive(Clone)]
struct Span {
    style: Style,
    text: String,
}

impl Span {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            style: Style::default(),
            text: text.into(),
        }
    }

    fn styled(style: Style, text: impl Into<String>) -> Self {
        Self {
            style,
            text: text.into(),
        }
    }
}

/// A finished block, ready to be wrapped and emitted.
enum Item {
    /// A run of inline content. `first`/`cont` are the prefixes for the first
    /// and any wrapped continuation line.
    Para {
        first: Vec<Span>,
        cont: Vec<Span>,
        body: Vec<Span>,
        list_item: bool,
    },
    /// A fenced or indented code block, emitted verbatim.
    Code {
        prefix: Vec<Span>,
        lines: Vec<String>,
    },
    /// A pipe table, laid out into aligned columns.
    Table {
        prefix: Vec<Span>,
        alignments: Vec<Alignment>,
        header_rows: usize,
        rows: Vec<Vec<Vec<Span>>>,
    },
    Rule,
}

#[derive(Default)]
struct ListCtx {
    ordered: bool,
    next: u64,
}

/// The prefix for the list item currently open. The indent is captured when the
/// item opens, since nested content can change the list depth before the item's
/// text is flushed.
struct ItemPrefix {
    indent: String,
    marker: String,
    marker_width: usize,
}

struct LinkCtx {
    url: String,
    body_start: usize,
}

/// Accumulates a pipe table. `TableHead` holds its cells directly, with no
/// `TableRow` wrapper, so the row is started lazily by the first cell.
#[derive(Default)]
struct TableBuilder {
    alignments: Vec<Alignment>,
    rows: Vec<Vec<Vec<Span>>>,
    header_rows: usize,
    row: Option<Vec<Vec<Span>>>,
    cell: Option<Vec<Span>>,
    in_head: bool,
}

impl TableBuilder {
    fn start_cell(&mut self) {
        self.row.get_or_insert_with(Vec::new);
        self.cell = Some(Vec::new());
    }

    fn end_cell(&mut self) {
        if let (Some(row), Some(cell)) = (self.row.as_mut(), self.cell.take()) {
            row.push(cell);
        }
    }

    fn start_row(&mut self) {
        self.row.get_or_insert_with(Vec::new);
    }

    fn finish_row(&mut self) {
        if let Some(row) = self.row.take()
            && !row.is_empty()
        {
            if self.in_head {
                self.header_rows += 1;
            }
            self.rows.push(row);
        }
    }
}

#[derive(Default)]
struct Layout {
    items: Vec<Item>,
    body: Vec<Span>,
    style: Style,
    style_stack: Vec<Style>,
    lists: Vec<ListCtx>,
    markers: Vec<ItemPrefix>,
    quotes: usize,
    links: Vec<LinkCtx>,
    code: Option<String>,
    table: Option<TableBuilder>,
}

impl Layout {
    fn run(mut self, markdown: &str) -> Vec<Item> {
        let source = sanitize(markdown);
        let options =
            Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES;
        for event in Parser::new_ext(&source, options) {
            self.event(event);
        }
        self.items
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.push_text(&text),
            Event::Code(text) => {
                let style = Style {
                    color: Some(CODE_COLOR),
                    ..self.style
                };
                self.push_span(Span::styled(style, text.to_string()));
            }
            Event::SoftBreak => self.push_span(Span::plain(" ")),
            Event::HardBreak => self.push_span(Span::plain("\n")),
            Event::Rule => self.items.push(Item::Rule),
            Event::TaskListMarker(done) => {
                let marker = if done { "☑ " } else { "☐ " };
                self.push_span(Span::styled(Style::dim(), marker));
            }
            Event::Html(html) | Event::InlineHtml(html) => self.push_text(&html),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                let underline = matches!(level, HeadingLevel::H1 | HeadingLevel::H2);
                self.push_style(|style| {
                    style.bold = true;
                    style.underline = underline;
                    style.color = Some(HEADING_COLOR);
                });
            }
            Tag::BlockQuote(_) => self.quotes += 1,
            Tag::CodeBlock(_) => self.code = Some(String::new()),
            Tag::Table(alignments) => {
                self.flush_para();
                self.table = Some(TableBuilder {
                    alignments,
                    ..TableBuilder::default()
                });
            }
            Tag::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.in_head = true;
                    table.start_row();
                }
            }
            Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.start_row();
                }
            }
            Tag::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.start_cell();
                }
            }
            Tag::List(start) => self.lists.push(ListCtx {
                ordered: start.is_some(),
                next: start.unwrap_or(1),
            }),
            Tag::Item => {
                // Tight list items have no `Paragraph`, so the previous item's
                // text is still pending when the next one opens.
                self.flush_para();
                let prefix = self.item_prefix();
                self.markers.push(prefix);
            }
            Tag::Emphasis => self.push_style(|style| style.italic = true),
            Tag::Strong => self.push_style(|style| style.bold = true),
            Tag::Strikethrough => self.push_style(|style| style.strike = true),
            Tag::Link { dest_url, .. } => {
                self.links.push(LinkCtx {
                    url: dest_url.to_string(),
                    body_start: self.inline_len(),
                });
                self.push_style(|style| {
                    style.underline = true;
                    style.color = Some(LINK_COLOR);
                });
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush_para(),
            TagEnd::Heading(_) => {
                self.flush_para();
                self.pop_style();
            }
            TagEnd::BlockQuote(_) => self.quotes = self.quotes.saturating_sub(1),
            TagEnd::CodeBlock => {
                let text = self.code.take().unwrap_or_default();
                let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
                while lines.last().is_some_and(String::is_empty) {
                    lines.pop();
                }
                self.items.push(Item::Code {
                    prefix: self.code_prefix(),
                    lines,
                });
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.items.push(Item::Table {
                        prefix: self.table_prefix(),
                        alignments: table.alignments,
                        header_rows: table.header_rows,
                        rows: table.rows,
                    });
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.finish_row();
                    table.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.finish_row();
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.end_cell();
                }
            }
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::Item => {
                self.flush_para();
                self.markers.pop();
            }
            TagEnd::Emphasis => self.pop_style(),
            TagEnd::Strong => self.pop_style(),
            TagEnd::Strikethrough => self.pop_style(),
            TagEnd::Link => {
                if let Some(link) = self.links.pop() {
                    let text = self.inline_text(link.body_start);
                    if text != link.url {
                        self.push_span(Span::styled(Style::dim(), format!(" ({})", link.url)));
                    }
                }
                self.pop_style();
            }
            _ => {}
        }
    }

    fn push_style(&mut self, apply: impl FnOnce(&mut Style)) {
        self.style_stack.push(self.style);
        apply(&mut self.style);
    }

    fn pop_style(&mut self) {
        if let Some(style) = self.style_stack.pop() {
            self.style = style;
        }
    }

    /// Send inline content to whichever buffer is currently open: a table
    /// cell, a code block, or the paragraph body.
    fn push_span(&mut self, span: Span) {
        if let Some(table) = self.table.as_mut()
            && let Some(cell) = table.cell.as_mut()
        {
            cell.push(span);
            return;
        }
        if let Some(buffer) = self.code.as_mut() {
            buffer.push_str(&span.text);
            return;
        }
        self.body.push(span);
    }

    fn push_text(&mut self, text: &str) {
        self.push_span(Span::styled(self.style, text.to_string()));
    }

    /// Number of inline spans accumulated in the currently open buffer.
    fn inline_len(&self) -> usize {
        match self.table.as_ref().and_then(|table| table.cell.as_ref()) {
            Some(cell) => cell.len(),
            None => self.body.len(),
        }
    }

    /// Text of the inline spans accumulated since `start`.
    fn inline_text(&self, start: usize) -> String {
        let spans = match self.table.as_ref().and_then(|table| table.cell.as_ref()) {
            Some(cell) => &cell[start.min(cell.len())..],
            None => &self.body[start.min(self.body.len())..],
        };
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    /// Prefix for the list item currently being opened.
    fn item_prefix(&mut self) -> ItemPrefix {
        let marker = match self.lists.last_mut() {
            Some(ctx) if ctx.ordered => {
                let marker = format!("{}. ", ctx.next);
                ctx.next += 1;
                marker
            }
            _ => "• ".to_string(),
        };
        ItemPrefix {
            indent: "  ".repeat(self.lists.len().saturating_sub(1)),
            marker_width: UnicodeWidthStr::width(marker.as_str()),
            marker,
        }
    }

    fn flush_para(&mut self) {
        if self.body.is_empty() {
            return;
        }
        let (first, cont) = self.prefixes();
        self.items.push(Item::Para {
            first,
            cont,
            body: std::mem::take(&mut self.body),
            list_item: !self.markers.is_empty(),
        });
    }

    /// Prefixes for the first and continuation lines of a paragraph.
    fn prefixes(&self) -> (Vec<Span>, Vec<Span>) {
        let mut first = self.quote_prefix();
        let mut cont = self.quote_prefix();
        if let Some(item) = self.markers.last() {
            first.push(Span::plain(item.indent.clone()));
            first.push(Span::styled(Style::dim(), item.marker.clone()));
            cont.push(Span::plain(item.indent.clone()));
            cont.push(Span::plain(" ".repeat(item.marker_width)));
        }
        (first, cont)
    }

    /// Quote gutters plus the content indent of the enclosing list item, so
    /// block content lines up under the item's text rather than its bullet.
    fn base_prefix(&self) -> Vec<Span> {
        let mut prefix = self.quote_prefix();
        if let Some(item) = self.markers.last() {
            prefix.push(Span::plain(format!(
                "{}{}",
                item.indent,
                " ".repeat(item.marker_width)
            )));
        }
        prefix
    }

    fn code_prefix(&self) -> Vec<Span> {
        let mut prefix = self.base_prefix();
        prefix.push(Span::plain("    "));
        prefix
    }

    fn table_prefix(&self) -> Vec<Span> {
        self.base_prefix()
    }

    fn quote_prefix(&self) -> Vec<Span> {
        (0..self.quotes)
            .map(|_| Span::styled(Style::dim(), "│ "))
            .collect()
    }
}

/// Drop control characters so model output cannot inject terminal escapes.
/// Newlines and tabs are preserved; `\r` and friends are not.
fn sanitize(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

fn emit(items: &[Item], width: usize, ansi: bool) -> String {
    let mut out = String::new();
    let mut prev_list_item = false;
    for item in items {
        let list_item = matches!(
            item,
            Item::Para {
                list_item: true,
                ..
            }
        );
        // Blank line between blocks, but not between consecutive list items.
        let continuing_list = list_item && prev_list_item;
        if !out.is_empty() && !continuing_list {
            out.push('\n');
        }
        match item {
            Item::Rule => {
                let rule = "─".repeat(width.min(60));
                emit_line(&mut out, &[Span::styled(Style::dim(), rule)], ansi);
            }
            Item::Code { prefix, lines } => {
                if lines.is_empty() {
                    emit_line(&mut out, prefix, ansi);
                }
                for line in lines {
                    let mut spans = prefix.clone();
                    spans.push(Span::plain(line.clone()));
                    emit_line(&mut out, &spans, ansi);
                }
            }
            Item::Para {
                first, cont, body, ..
            } => {
                let budget = width.saturating_sub(visible_width(first)).max(10);
                let lines = wrap(body, budget);
                if lines.is_empty() {
                    emit_line(&mut out, first, ansi);
                }
                for (index, line) in lines.iter().enumerate() {
                    let prefix = if index == 0 { first } else { cont };
                    let mut spans = prefix.clone();
                    spans.extend(line.iter().cloned());
                    emit_line(&mut out, &spans, ansi);
                }
            }
            Item::Table {
                prefix,
                alignments,
                header_rows,
                rows,
            } => emit_table(
                &mut out,
                prefix,
                alignments,
                *header_rows,
                rows,
                width,
                ansi,
            ),
        }
        prev_list_item = list_item;
    }
    out
}

/// Lay out a pipe table into aligned columns, wrapping cell content that does
/// not fit and padding each cell according to its column alignment.
fn emit_table(
    out: &mut String,
    prefix: &[Span],
    alignments: &[Alignment],
    header_rows: usize,
    rows: &[Vec<Vec<Span>>],
    width: usize,
    ansi: bool,
) {
    let columns = rows
        .iter()
        .map(Vec::len)
        .chain(std::iter::once(alignments.len()))
        .max()
        .unwrap_or(0);
    if columns == 0 {
        return;
    }

    let mut widths = vec![0usize; columns];
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(visible_width(cell));
        }
    }

    // " │ " between columns, plus the prefix already on every line.
    let separators = 3 * columns.saturating_sub(1);
    let available = width.saturating_sub(visible_width(prefix) + separators);
    fit_widths(&mut widths, available);

    let gutter = Span::styled(Style::dim(), " │ ");
    for (index, row) in rows.iter().enumerate() {
        let header = index < header_rows;
        let wrapped: Vec<Vec<Vec<Span>>> = (0..columns)
            .map(|column| {
                let cell = row.get(column).cloned().unwrap_or_default();
                let cell = if header { bolden(&cell) } else { cell };
                wrap(&cell, widths[column].max(1))
            })
            .collect();

        let height = wrapped.iter().map(Vec::len).max().unwrap_or(0).max(1);
        for line in 0..height {
            let mut spans = prefix.to_vec();
            for column in 0..columns {
                if column > 0 {
                    spans.push(gutter.clone());
                }
                let empty: Vec<Span> = Vec::new();
                let content = wrapped[column].get(line).unwrap_or(&empty);
                let alignment = alignments.get(column).copied().unwrap_or(Alignment::None);
                spans.extend(pad(content, widths[column], alignment));
            }
            emit_line(out, &spans, ansi);
        }

        if header && index + 1 == header_rows {
            let mut spans = prefix.to_vec();
            for (column, column_width) in widths.iter().enumerate() {
                if column > 0 {
                    spans.push(Span::styled(Style::dim(), "─┼─"));
                }
                spans.push(Span::styled(Style::dim(), "─".repeat(*column_width)));
            }
            emit_line(out, &spans, ansi);
        }
    }
}

/// Shrink columns to fit `available` visible columns, proportionally and never
/// below [`MIN_COLUMN`]. If the minimum cannot fit, the table overflows.
fn fit_widths(widths: &mut [usize], available: usize) {
    const MIN_COLUMN: usize = 8;
    let total: usize = widths.iter().sum();
    if total <= available {
        return;
    }
    let mut fitted: Vec<usize> = widths
        .iter()
        .map(|column| (*column * available / total).max(MIN_COLUMN))
        .collect();
    let mut excess = fitted.iter().sum::<usize>().saturating_sub(available);
    while excess > 0 {
        let widest = fitted
            .iter()
            .enumerate()
            .filter(|(_, column)| **column > MIN_COLUMN)
            .max_by_key(|(_, column)| **column)
            .map(|(index, _)| index);
        let Some(index) = widest else { break };
        let take = (fitted[index] - MIN_COLUMN).min(excess);
        fitted[index] -= take;
        excess -= take;
    }
    widths.copy_from_slice(&fitted);
}

/// Pad spans to exactly `width` visible columns.
fn pad(spans: &[Span], width: usize, alignment: Alignment) -> Vec<Span> {
    let content = visible_width(spans);
    if content >= width {
        return spans.to_vec();
    }
    let padding = width - content;
    let (left, right) = match alignment {
        Alignment::Right => (padding, 0),
        Alignment::Center => (padding / 2, padding - padding / 2),
        Alignment::None | Alignment::Left => (0, padding),
    };
    let mut out = Vec::new();
    if left > 0 {
        out.push(Span::plain(" ".repeat(left)));
    }
    out.extend(spans.iter().cloned());
    if right > 0 {
        out.push(Span::plain(" ".repeat(right)));
    }
    out
}

fn bolden(spans: &[Span]) -> Vec<Span> {
    spans
        .iter()
        .map(|span| {
            Span::styled(
                Style {
                    bold: true,
                    ..span.style
                },
                span.text.clone(),
            )
        })
        .collect()
}

fn emit_line(out: &mut String, spans: &[Span], ansi: bool) {
    for span in spans {
        if ansi && span.style != Style::default() {
            out.push_str(&sgr(span.style));
            out.push_str(&span.text);
            out.push_str(RESET);
        } else {
            out.push_str(&span.text);
        }
    }
    out.push('\n');
}

fn sgr(style: Style) -> String {
    let mut codes = String::new();
    if style.bold {
        codes.push_str("1;");
    }
    if style.dim {
        codes.push_str("2;");
    }
    if style.italic {
        codes.push_str("3;");
    }
    if style.underline {
        codes.push_str("4;");
    }
    if style.strike {
        codes.push_str("9;");
    }
    if let Some(color) = style.color {
        let _ = write!(codes, "38;5;{color};");
    }
    codes.pop();
    format!("\x1b[{codes}m")
}

fn visible_width(spans: &[Span]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.text.as_str()))
        .sum()
}

/// Greedily wrap spans to `width` visible columns, breaking on whitespace.
/// `\n` forces a break; words longer than the width are allowed to overflow.
fn wrap(spans: &[Span], width: usize) -> Vec<Vec<Span>> {
    let mut lines: Vec<Vec<Span>> = Vec::new();
    let mut current: Vec<Span> = Vec::new();
    let mut current_width = 0usize;

    for span in spans {
        for (index, part) in span.text.split('\n').enumerate() {
            if index > 0 {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
            }
            for (is_space, chunk) in runs(part) {
                let chunk_width = UnicodeWidthStr::width(chunk);
                if is_space {
                    if current_width == 0 {
                        continue;
                    }
                    if current_width + chunk_width > width {
                        lines.push(std::mem::take(&mut current));
                        current_width = 0;
                        continue;
                    }
                } else if current_width > 0 && current_width + chunk_width > width {
                    lines.push(std::mem::take(&mut current));
                    current_width = 0;
                }
                push_span(&mut current, span.style, chunk);
                current_width += chunk_width;
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Split text into alternating whitespace and non-whitespace runs.
fn runs(text: &str) -> Vec<(bool, &str)> {
    let mut out: Vec<(bool, &str)> = Vec::new();
    let mut start = 0;
    let mut current: Option<bool> = None;
    for (index, c) in text.char_indices() {
        let is_space = c.is_whitespace();
        match current {
            Some(previous) if previous == is_space => {}
            Some(previous) => {
                out.push((previous, &text[start..index]));
                start = index;
                current = Some(is_space);
            }
            None => current = Some(is_space),
        }
    }
    if let Some(previous) = current {
        out.push((previous, &text[start..]));
    }
    out
}

fn push_span(spans: &mut Vec<Span>, style: Style, text: &str) {
    if let Some(last) = spans.last_mut()
        && last.style == style
    {
        last.text.push_str(text);
        return;
    }
    spans.push(Span::styled(style, text.to_string()));
}

#[cfg(test)]
mod tests {
    use super::render;
    use unicode_width::UnicodeWidthStr;

    /// Strip SGR sequences so assertions can be written against visible text.
    fn strip(input: &str) -> String {
        let mut out = String::new();
        let mut chars = input.chars();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        }
        out
    }

    fn lines(markdown: &str, width: usize) -> Vec<String> {
        strip(&render(markdown, width, true))
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn empty_input_renders_nothing() {
        assert_eq!(render("", 40, true), "");
    }

    #[test]
    fn heading_is_styled_and_not_wrapped_away() {
        let out = render("## Title", 40, true);
        assert!(out.contains("Title"));
        assert!(out.contains("\x1b[1;4;38;5;39m"), "{out:?}");
    }

    #[test]
    fn plain_mode_emits_no_escapes() {
        let out = render("# Title\n\n**bold** and `code`", 40, false);
        assert!(!out.contains('\x1b'));
        assert!(out.contains("Title"));
        assert!(out.contains("bold"));
    }

    #[test]
    fn paragraphs_are_separated_by_a_blank_line() {
        let out = lines("first\n\nsecond", 40);
        assert_eq!(out, vec!["first", "", "second"]);
    }

    #[test]
    fn inline_styles_are_emitted() {
        let out = render("**bold** *italic* `code` ~~struck~~", 60, true);
        assert!(out.contains("\x1b[1m"), "bold: {out:?}");
        assert!(out.contains("\x1b[3m"), "italic: {out:?}");
        assert!(out.contains("38;5;214"), "code: {out:?}");
        assert!(out.contains("\x1b[9m"), "strike: {out:?}");
    }

    #[test]
    fn wrapped_lines_fit_within_width() {
        let markdown = "The quick brown fox jumps over the lazy dog and keeps running for a while.";
        let rendered = render(markdown, 30, true);
        for line in strip(&rendered).lines() {
            assert!(
                UnicodeWidthStr::width(line) <= 30,
                "line too wide: {line:?}"
            );
        }
    }

    #[test]
    fn bullet_list_wraps_with_hanging_indent() {
        let out = lines("- alpha beta gamma delta epsilon zeta", 20);
        assert!(out[0].starts_with("• alpha"));
        assert!(out.len() > 1, "expected wrapping: {out:?}");
        for line in &out[1..] {
            assert!(
                line.starts_with("  "),
                "continuation not indented: {line:?}"
            );
        }
    }

    #[test]
    fn ordered_list_numbers_items() {
        let out = lines("1. one\n2. two\n3. three", 40);
        assert_eq!(out[0], "1. one");
        assert_eq!(out[1], "2. two");
        assert_eq!(out[2], "3. three");
    }

    #[test]
    fn nested_list_is_indented() {
        let out = lines("- outer\n  - inner", 40);
        assert_eq!(out[0], "• outer");
        assert!(out[1].starts_with("  • inner"), "{out:?}");
    }

    #[test]
    fn list_items_are_not_separated_by_blank_lines() {
        let out = lines("- one\n- two", 40);
        assert_eq!(out, vec!["• one", "• two"]);
    }

    #[test]
    fn code_block_is_preserved_verbatim() {
        let markdown = "```\nfn main() {\n    let x = 1;\n}\n```";
        let out = lines(markdown, 20);
        assert_eq!(out[0], "    fn main() {");
        assert_eq!(out[1], "        let x = 1;");
        assert_eq!(out[2], "    }");
    }

    #[test]
    fn long_code_lines_are_not_wrapped() {
        let long = "x".repeat(120);
        let markdown = format!("```\n{long}\n```");
        let out = lines(&markdown, 20);
        assert!(out.iter().any(|line| line.contains(&long)));
    }

    #[test]
    fn code_block_inside_a_list_item_is_indented_under_the_text() {
        let markdown = "- item\n\n  ```\n  code\n  ```";
        let out = lines(markdown, 40);
        assert_eq!(out[0], "• item");
        assert_eq!(out[2], "      code");
    }

    #[test]
    fn blockquote_uses_a_gutter() {
        let out = lines("> quoted text", 40);
        assert_eq!(out[0], "│ quoted text");
    }

    #[test]
    fn link_renders_text_and_url() {
        let out = lines("see [the docs](https://example.com/a)", 60);
        assert!(out[0].contains("the docs"));
        assert!(out[0].contains("(https://example.com/a)"));
    }

    #[test]
    fn autolink_does_not_duplicate_the_url() {
        let out = lines("<https://example.com>", 60);
        assert_eq!(out[0], "https://example.com");
    }

    #[test]
    fn task_list_markers_render() {
        let out = lines("- [x] done\n- [ ] todo", 40);
        assert!(out[0].contains("☑ done"), "{out:?}");
        assert!(out[1].contains("☐ todo"), "{out:?}");
    }

    #[test]
    fn rule_renders_a_divider() {
        let out = lines("a\n\n---\n\nb", 40);
        assert!(out.iter().any(|line| line.contains("───")), "{out:?}");
    }

    #[test]
    fn table_aligns_columns_and_pads() {
        let markdown = "| Name | Age |\n|:-----|----:|\n| Alice | 30 |\n| Bob | 7 |";
        let out = lines(markdown, 60);
        assert_eq!(out[0], "Name  │ Age");
        assert_eq!(out[1], "──────┼────");
        assert_eq!(out[2], "Alice │  30");
        assert_eq!(out[3], "Bob   │   7");
    }

    #[test]
    fn table_centers_when_asked() {
        let markdown = "| a | b |\n|:--|:-:|\n| 1 | 2 |";
        let out = lines(markdown, 40);
        assert_eq!(out[0], "a │ b");
        assert_eq!(out[2], "1 │ 2");
    }

    #[test]
    fn table_header_is_bold() {
        let out = render("| Name |\n|:-----|\n| Alice |", 40, true);
        assert!(out.contains("\x1b[1mName"), "{out:?}");
    }

    #[test]
    fn wide_table_wraps_cells_within_width() {
        let markdown = "\
| Key | Description |
|:----|:------------|
| alpha | a fairly long description that will not fit in a narrow terminal |
| beta | short |";
        let rendered = render(markdown, 40, true);
        for line in strip(&rendered).lines() {
            assert!(UnicodeWidthStr::width(line) <= 40, "too wide: {line:?}");
        }
    }

    #[test]
    fn table_inside_a_list_item_is_indented() {
        let markdown = "- item\n\n  | a | b |\n  |:--|:--|\n  | 1 | 2 |";
        let out = lines(markdown, 40);
        let row = out.iter().find(|line| line.contains("1 │ 2")).unwrap();
        assert!(row.starts_with("  "), "{out:?}");
    }

    #[test]
    fn table_with_inline_styles_keeps_them() {
        let markdown = "| a |\n|:--|\n| **bold** |";
        let out = render(markdown, 40, true);
        assert!(out.contains("\x1b[1mbold"), "{out:?}");
    }

    #[test]
    fn table_cells_keep_inline_code() {
        let markdown = "| cmd |\n|:----|\n| `ls -la` |";
        let out = lines(markdown, 40);
        assert_eq!(out[2], "ls -la");
    }

    #[test]
    fn control_characters_are_stripped() {
        let out = render("safe\x1b[31mred\x07text", 40, false);
        assert!(!out.contains('\x1b'), "{out:?}");
        assert!(!out.contains('\x07'), "{out:?}");
        assert_eq!(out.trim_end(), "safe[31mredtext");
    }

    #[test]
    fn styles_do_not_consume_wrap_budget() {
        // "**aaaa** bbbb" is 9 visible columns; styling must not push it over.
        let rendered = render("**aaaa** bbbb", 9, true);
        assert_eq!(strip(&rendered).trim_end(), "aaaa bbbb");
        assert_eq!(strip(&rendered).lines().count(), 1);
    }
}

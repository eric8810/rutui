//! Bash-command display: wrapping + syntax highlighting for tool headers.
//!
//! W4 decoupling: inlined from the pager's
//! `views/{tasks_pane,permission_view}.rs` and `xai-grok-workspace`'s
//! `permission::bash_command_splitting` (the scrollback execute/bg-task
//! headers render commands exactly the way the permission overlay does).
//! All of it is pure text work (tree-sitter-bash operator awareness,
//! quote-aware width wrap, one stateful syntect shell highlighter); the
//! xai-grok-workspace copy remains authoritative for the permission overlay
//! itself. If the two drift, re-sync from those origins.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tree_sitter::{Node, Parser, Tree};
use unicode_width::UnicodeWidthStr;

use crate::syntax::{get_syntect, syntect_to_ratatui_fg};
use rutui_theme::theme::Theme;

// ---------------------------------------------------------------------------
// Syntax-highlighted single-line command header (from views/tasks_pane.rs)
// ---------------------------------------------------------------------------

/// Highlight a shell command string into styled spans.
///
/// Uses syntect with the best available grammar for the platform: tries
/// "powershell" first on Windows, falls back to "bash". Returns plain
/// `theme.command` color if no grammar matches. Results should be cached.
pub fn highlight_bash_command(command: &str) -> Vec<Span<'static>> {
    let syntect = get_syntect();
    let grammar = if cfg!(windows) { "powershell" } else { "bash" };
    let Some(mut hl) = syntect
        .highlight_lines_for_token(grammar)
        .or_else(|| syntect.highlight_lines_for_token("bash"))
    else {
        let theme = Theme::current();
        return vec![Span::styled(
            command.to_string(),
            Style::default().fg(theme.command),
        )];
    };

    let line = format!("{command}\n");
    match hl.highlight_line(&line, &syntect.syntax_set) {
        Ok(ranges) => {
            let mut spans = Vec::new();
            for (style, segment) in ranges {
                let mut text = segment.to_owned();
                while text.ends_with('\n') || text.ends_with('\r') {
                    text.pop();
                }
                if text.is_empty() {
                    continue;
                }
                spans.push(Span::styled(text, syntect_to_ratatui_fg(style)));
            }
            if spans.is_empty() {
                let theme = Theme::current();
                vec![Span::styled(
                    command.to_string(),
                    Style::default().fg(theme.command),
                )]
            } else {
                spans
            }
        }
        Err(_) => {
            let theme = Theme::current();
            vec![Span::styled(
                command.to_string(),
                Style::default().fg(theme.command),
            )]
        }
    }
}

// ---------------------------------------------------------------------------
// Bash command display lines (from views/permission_view.rs)
// ---------------------------------------------------------------------------

/// Wrap + syntax-highlight a bash command the same way the permission
/// overlay body does: preserve source newlines / `\` continuations, keep
/// heredoc bodies intact, quote-aware width wrap only — **no** soft-breaks
/// at `&&` / `||` / `|` / `;` (those made one command look like multiple
/// prompts once the full command is shown in the overlay).
///
/// Used by the execute tool-call header so scrollback matches the overlay.
pub fn render_bash_command_display_lines(
    command: &str,
    content_width: usize,
) -> Vec<Line<'static>> {
    build_raw_bash_lines(command, content_width, usize::MAX)
}

/// Normalize command text for display without destroying structure.
///
/// - Unifies line endings to `\n`
/// - Trims trailing whitespace per physical line (keeps indent)
/// - **Preserves** intentional newlines, including lines that end in `\`
///   (shell line continuations like `cmd \\\n  --flag`)
fn prepare_bash_display_text(command: &str) -> String {
    let normalized = command.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(normalized.len());
    for (i, line) in normalized.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line.trim_end());
    }
    // Drop trailing blank lines (common when scripts end with `\n`)
    // but keep interior blank lines.
    while out.ends_with('\n') {
        let without = &out[..out.len() - 1];
        if without.ends_with('\\') {
            // Dangling `\` continuation at EOF: keep the backslash visible on
            // the last line but drop the now-useless trailing newline, which
            // would otherwise render as a stray empty row in the overlay.
            out.pop();
            break;
        }
        if without.is_empty() || without.ends_with('\n') {
            out.pop();
            continue;
        }
        // Single trailing newline after content — drop it.
        out.pop();
        break;
    }
    out
}

/// Soft-wrap one physical line into contiguous byte slice-rows, breaking at
/// tree-sitter-validated shell operators (`&&` / `||` / `|` / `;`) first,
/// then quote-aware within each segment; heredoc payload lines stay intact.
fn soft_wrap_row_texts<'a>(
    line: &'a str,
    line_start: usize,
    full_breaks: &[usize],
    heredoc_payload: &[(usize, usize)],
    content_width: usize,
    max_rows: usize,
) -> Vec<&'a str> {
    if max_rows == 0 {
        return Vec::new();
    }
    if content_width == 0 {
        return vec![line];
    }

    // Bytes bound columns (a char's width never exceeds its UTF-8 length),
    // so a byte-short line provably fits without any width scan; longer
    // lines are resolved by the exact per-candidate checks below.
    if line.len() <= content_width {
        return vec![line];
    }

    // Heredoc body/content is free-form payload, not shell syntax — do not
    // soft-wrap at spaces. Keep the physical line intact even if it overflows.
    let line_end = line_start + line.len();
    if range_fully_inside(line_start, line_end, heredoc_payload) {
        return vec![line];
    }

    // Chunk-end boundaries: line-relative operator soft-breaks strictly
    // inside the line (same filter as the workspace's
    // `split_physical_line_at_soft_breaks`, whose global offsets arrive
    // sorted and deduped), then the line end — discovered lazily so a capped
    // wrap never materializes the chunk list of a huge operator chain.
    let first_inside = full_breaks.partition_point(|&b| b <= line_start);
    let mut bounds = full_breaks[first_inside..]
        .iter()
        .copied()
        .take_while(|&b| b < line_end)
        .map(|b| b - line_start)
        .filter(|&b| line.is_char_boundary(b))
        .chain(std::iter::once(line.len()))
        .peekable();

    // No real operators on this line (or parse found none) — quote-aware wrap.
    if bounds.peek().copied() == Some(line.len()) {
        return bash_quote_aware_wrap(line, content_width, max_rows);
    }

    // Fused pack+emit: extend each row over whole chunks while it fits,
    // emit the trimmed row (quote-wrapping a chunk that alone overflows),
    // and skip the inter-command whitespace before each continuation row.
    let mut out: Vec<&'a str> = Vec::new();
    let mut pos = 0usize;
    let mut first_row = true;
    while pos < line.len() && out.len() < max_rows {
        // Skip leading whitespace when *starting* a continuation after a
        // previous row — that space belonged between operator and next cmd.
        let mut start = pos;
        if !first_row {
            while start < line.len() && line.as_bytes()[start].is_ascii_whitespace() {
                start += 1;
            }
            // Drop boundaries at or before the skipped whitespace.
            while bounds.peek().is_some_and(|&b| b <= start) {
                bounds.next();
            }
            if start >= line.len() {
                break;
            }
        }
        first_row = false;

        let Some(mut end) = bounds.next() else {
            break;
        };
        // Display width from `start` (whitespace-trimmed for continuations).
        if UnicodeWidthStr::width(&line[start..end]) <= content_width {
            // Pack subsequent whole chunks while the row still fits.
            while let Some(&next_end) = bounds.peek() {
                if UnicodeWidthStr::width(&line[start..next_end]) <= content_width {
                    end = next_end;
                    bounds.next();
                } else {
                    break;
                }
            }
            // First row keeps any intentional indent; continuations had
            // leading ws skipped via `start`.
            out.push(line[start..end].trim_end());
        } else {
            // Chunk alone exceeds width — quote-wrap the rest of this chunk.
            let row = line[start..end].trim_end();
            if UnicodeWidthStr::width(row) <= content_width {
                out.push(row);
            } else {
                out.extend(bash_quote_aware_wrap(
                    row,
                    content_width,
                    max_rows - out.len(),
                ));
            }
        }
        pos = end;
    }
    out
}

/// Word-wrap a bash fragment without breaking on whitespace that sits inside
/// single- or double-quoted strings.
///
/// Break candidates are byte offsets *after* a run of whitespace that is not
/// inside quotes. If a single unbreakable span (e.g. a long `'...'` literal)
/// still exceeds `width`, it is emitted as one row (may overflow the panel —
/// better than splitting `jq '.[] | ...'` mid-expression).
///
/// At most `max_rows` rows are produced; the scan returns as soon as the cap
/// is reached and break points are discovered lazily, so a huge unquoted
/// line costs only the candidate rows actually considered — never a
/// full-line width scan or a full break-offset allocation.
fn bash_quote_aware_wrap(line: &str, width: usize, max_rows: usize) -> Vec<&str> {
    if max_rows == 0 {
        return Vec::new();
    }
    // Bytes bound columns (a char's width never exceeds its UTF-8 length),
    // so a byte-short fragment provably fits without any width scan; longer
    // fragments are resolved by the exact per-candidate checks below.
    if width == 0 || line.len() <= width {
        return vec![line];
    }

    // Lazy candidate stream: quote-aware break offsets, then EOL. Nothing
    // past the last consumed candidate is ever scanned.
    let mut break_points = QuoteAwareBreakPoints::new(line).peekable();
    if break_points.peek().is_none() {
        // Nowhere safe to break (entire line is one quoted span, or no spaces).
        return vec![line];
    }

    let mut rows: Vec<&str> = Vec::new();
    let mut row_start = 0usize;
    let mut last_break = 0usize; // exclusive end of content if we break here

    // Consider each break point as a candidate end for the current row.
    let candidates = break_points.chain(std::iter::once(line.len())); // allow ending at EOL

    for b in candidates {
        if b <= row_start {
            continue;
        }
        let candidate = line[row_start..b].trim_end();
        if UnicodeWidthStr::width(candidate) <= width {
            last_break = b;
            continue;
        }
        // Exceeded width: emit up to last_break if we made progress.
        if last_break > row_start {
            let row = line[row_start..last_break].trim_end();
            if !row.is_empty() {
                rows.push(row);
                if rows.len() >= max_rows {
                    return rows;
                }
            }
            // Next row starts after whitespace at last_break.
            row_start = last_break;
            while row_start < line.len() && line.as_bytes()[row_start].is_ascii_whitespace() {
                row_start += 1;
            }
            last_break = row_start;
            // Re-evaluate this break point against the new row_start.
            if b > row_start {
                let candidate = line[row_start..b].trim_end();
                if UnicodeWidthStr::width(candidate) <= width {
                    last_break = b;
                } else {
                    // Still too wide with nothing smaller — force-emit unbreakable.
                    let force_end = b;
                    let row = line[row_start..force_end].trim_end();
                    if !row.is_empty() {
                        rows.push(row);
                        if rows.len() >= max_rows {
                            return rows;
                        }
                    }
                    row_start = force_end;
                    while row_start < line.len() && line.as_bytes()[row_start].is_ascii_whitespace()
                    {
                        row_start += 1;
                    }
                    last_break = row_start;
                }
            }
        } else {
            // No prior break in this row — unbreakable span larger than width.
            let row = line[row_start..b].trim_end();
            if !row.is_empty() {
                rows.push(row);
                if rows.len() >= max_rows {
                    return rows;
                }
            }
            row_start = b;
            while row_start < line.len() && line.as_bytes()[row_start].is_ascii_whitespace() {
                row_start += 1;
            }
            last_break = row_start;
        }
    }
    if row_start < line.len() && rows.len() < max_rows {
        let row = line[row_start..].trim_end();
        if !row.is_empty() {
            rows.push(row);
        }
    }
    if rows.is_empty() { vec![line] } else { rows }
}

/// Lazy iterator over byte offsets at the *start* of whitespace runs that
/// are safe soft-wrap points (outside single/double quotes). The caller ends
/// the current row at this offset (trimming the run) and skips the
/// whitespace before the next row. Offsets inside quotes are never yielded;
/// yields are strictly increasing.
///
/// Lazy so a capped [`bash_quote_aware_wrap`] never scans (or allocates)
/// break points past the ones its visible rows actually consume.
struct QuoteAwareBreakPoints<'a> {
    bytes: &'a [u8],
    i: usize,
    in_single: bool,
    in_double: bool,
}

impl<'a> QuoteAwareBreakPoints<'a> {
    fn new(line: &'a str) -> Self {
        Self {
            bytes: line.as_bytes(),
            i: 0,
            in_single: false,
            in_double: false,
        }
    }
}

impl Iterator for QuoteAwareBreakPoints<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        while self.i < self.bytes.len() {
            let c = self.bytes[self.i];
            if self.in_single {
                if c == b'\'' {
                    self.in_single = false;
                }
                self.i += 1;
                continue;
            }
            if self.in_double {
                if c == b'\\' && self.i + 1 < self.bytes.len() {
                    self.i += 2; // skip escape
                    continue;
                }
                if c == b'"' {
                    self.in_double = false;
                }
                self.i += 1;
                continue;
            }
            match c {
                b'\'' => {
                    self.in_single = true;
                    self.i += 1;
                }
                b'"' => {
                    self.in_double = true;
                    self.i += 1;
                }
                b if b.is_ascii_whitespace() => {
                    // Consume the whole whitespace run; break *after* it so
                    // the next row starts at non-ws (caller also trims).
                    let start = self.i;
                    while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_whitespace() {
                        self.i += 1;
                    }
                    // Prefer breaking after the whitespace (start of next
                    // token); `start` lets the previous row end before it.
                    if start > 0 {
                        return Some(start);
                    }
                }
                _ => self.i += 1,
            }
        }
        None
    }
}

/// Build syntax-highlighted lines for a (possibly multi-line) bash command,
/// stopping after `max_rows` wrap rows.
///
/// Preserves intentional newlines / `\` continuations. Uses **one** stateful
/// shell highlighter across all physical lines (so heredocs, open quotes, and
/// continuations keep their lexer state), advanced exactly once per physical
/// `\n` line — in order, and only until `max_rows` rows exist, so a collapsed
/// huge script is never fully highlighted. Overlong lines soft-wrap at
/// tree-sitter-validated shell operators (`&&` / `||` / `|` / `;`), then
/// quote-aware width wrap within each segment — wrap rows are sliced out of
/// the already-highlighted spans and are never re-lexed.
fn build_raw_bash_lines(
    command: &str,
    content_width: usize,
    max_rows: usize,
) -> Vec<Line<'static>> {
    let text = prepare_bash_display_text(command);
    if text.is_empty() || max_rows == 0 {
        return Vec::new();
    }

    // Operator breaks + heredoc ranges need a full-script parse so body lines
    // are not space-wrapped and quoted `&&` is not treated as a list op.
    let full_breaks = soft_break_offsets_after_operators(&text);
    let heredoc_payload = heredoc_payload_byte_ranges(&text);

    let syntect = get_syntect();
    let fallback = Style::default().fg(Theme::current().command);
    let grammar = if cfg!(windows) { "powershell" } else { "bash" };
    let mut hl = syntect
        .highlight_lines_for_token(grammar)
        .or_else(|| syntect.highlight_lines_for_token("bash"));

    let mut out = Vec::new();
    let mut offset = 0usize;
    for (idx, physical) in text.split('\n').enumerate() {
        // Row budget filled — skip highlighting the rest of the script.
        if out.len() >= max_rows {
            break;
        }
        if idx > 0 {
            offset += 1; // the '\n'
        }
        // Advance the shared lexer state exactly once per physical line —
        // blank lines included, so multi-line constructs stay in sync.
        let spans = crate::syntax::highlight_line(physical, &mut hl, syntect, fallback);
        if physical.is_empty() {
            out.push(Line::default());
            continue;
        }
        debug_assert_eq!(
            spans.iter().map(|s| s.content.as_ref()).collect::<String>(),
            physical,
            "highlight spans must flatten back to the physical line"
        );
        // Cap-aware wrap: only the rows still inside the budget are produced
        // and sliced, even within one huge physical line.
        for row in soft_wrap_row_texts(
            physical,
            offset,
            &full_breaks,
            &heredoc_payload,
            content_width,
            max_rows - out.len(),
        ) {
            // Rows are sub-slices of `physical`; recover the byte range.
            let start = (row.as_ptr() as usize) - (physical.as_ptr() as usize);
            out.push(Line::from(slice_highlighted_spans(
                &spans,
                start,
                start + row.len(),
            )));
        }
        offset += physical.len();
    }
    out
}

/// Slice already-highlighted spans down to the byte range `[start, end)` of
/// the physical line they were produced from, preserving each span's style.
///
/// Boundaries come from `&str` sub-slices of that same line, so they are
/// always char-aligned; a mid-char boundary would indicate a broken caller
/// invariant and the affected span is skipped rather than panicking.
fn slice_highlighted_spans(
    spans: &[Span<'static>],
    start: usize,
    end: usize,
) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    for span in spans {
        let span_start = pos;
        let span_end = pos + span.content.len();
        pos = span_end;
        if span_end <= start {
            continue;
        }
        if span_start >= end {
            break;
        }
        let lo = start.max(span_start) - span_start;
        let hi = end.min(span_end) - span_start;
        if lo >= hi {
            continue;
        }
        let Some(slice) = span.content.get(lo..hi) else {
            debug_assert!(false, "row boundary off a char boundary");
            continue;
        };
        out.push(Span::styled(slice.to_owned(), span.style));
    }
    out
}

// ---------------------------------------------------------------------------
// Tree-sitter shell analysis (from xai-grok-workspace::permission::bash_command_splitting)
// ---------------------------------------------------------------------------

/// Parse the provided bash source using tree-sitter-bash, returning a Tree on
/// success or None if parsing failed.
pub fn try_parse_shell(src: &str) -> Option<Tree> {
    let lang = tree_sitter_bash::LANGUAGE.into();
    let mut parser = Parser::new();
    #[expect(clippy::expect_used)]
    parser.set_language(&lang).expect("load bash grammar");
    let old_tree: Option<&Tree> = None;
    parser.parse(src, old_tree)
}

/// Node kinds whose payload must not be descended into when looking for
/// soft-break operators (heredoc bodies, quoted strings, comments).
const PAYLOAD_NODE_KINDS: &[&str] = &[
    // Heredoc body / content (not the redirect operator itself on the
    // command line — `cat <<EOF && true` still has a real list `&&`).
    "heredoc_body",
    "simple_heredoc_body",
    "heredoc_content",
    "heredoc_end",
    // Quoted / expansion payload
    "string",
    "raw_string",
    "string_content",
    "ansi_c_string",
    "translated_string",
    // Comments
    "comment",
];

/// True when `kind` is a shell list/pipeline operator we may soft-break after.
fn is_soft_break_operator_kind(kind: &str) -> bool {
    matches!(kind, "&&" | "||" | "|" | ";")
}

/// True when we must not descend into this node looking for operators.
fn is_payload_node_kind(kind: &str) -> bool {
    PAYLOAD_NODE_KINDS.contains(&kind)
}

/// Byte offsets into `script` **after** real shell list/pipeline operators
/// where a display soft-wrap is safe.
///
/// Uses tree-sitter-bash so `&&` / `||` / `|` / `;` that appear only inside
/// strings, heredoc bodies, or comments are **not** returned. The command-line
/// operator in `cat <<EOF && echo after` **is** returned (it is a real `list`
/// operator); the body's `foo && bar` is not.
///
/// Returns an empty vec when the script cannot be parsed at all (caller should
/// fall back to width-only word-wrap, not naive substring splits).
///
/// Offsets are sorted ascending and de-duplicated. Each offset is
/// `operator_node.end_byte()` — i.e. the split keeps the operator on the
/// preceding display row.
pub fn soft_break_offsets_after_operators(script: &str) -> Vec<usize> {
    let Some(tree) = try_parse_shell(script) else {
        return Vec::new();
    };

    let root = tree.root_node();
    // On a broken parse, tree-sitter can still expose `|` / `&&` / `;` nodes
    // that are *not* real shell control flow (e.g. fragments of unclosed
    // strings or half-parsed heredocs). Prefer no soft-breaks over wrong ones.
    if root.has_error() {
        return Vec::new();
    }

    let mut breaks: Vec<usize> = Vec::new();
    let mut stack: Vec<Node> = vec![root];

    while let Some(node) = stack.pop() {
        let kind = node.kind();

        // Do not walk into string / heredoc / comment payload — any operator
        // characters there are not shell syntax nodes we care about, and
        // skipping the whole subtree is cheaper and safer.
        if is_payload_node_kind(kind) {
            continue;
        }

        if is_soft_break_operator_kind(kind) {
            let end = node.end_byte();
            if end > 0 && end <= script.len() && script.is_char_boundary(end) {
                breaks.push(end);
            }
        }

        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }

    breaks.sort_unstable();
    breaks.dedup();
    breaks
}

/// Byte ranges of heredoc *payload* (body / content), not the `<<WORD` opener
/// on the command line.
///
/// Used so physical lines that are pure heredoc body text are **not**
/// soft-wrapped at spaces (they are free-form payload, not shell syntax).
/// Returns an empty vec when the script cannot be parsed or the tree has
/// errors (same policy as [`soft_break_offsets_after_operators`]: error
/// recovery can invent bogus heredoc spans).
pub fn heredoc_payload_byte_ranges(script: &str) -> Vec<(usize, usize)> {
    let Some(tree) = try_parse_shell(script) else {
        return Vec::new();
    };

    let root = tree.root_node();
    // Match soft-break policy: on a broken parse, tree-sitter error recovery can
    // invent or mis-bound `heredoc_body` nodes. Prefer no payload ranges (normal
    // soft-wrap / no false no-wrap) over wrong spans that overflow or skip wraps.
    if root.has_error() {
        return Vec::new();
    }

    const HEREDOC_PAYLOAD: &[&str] = &["heredoc_body", "simple_heredoc_body", "heredoc_content"];

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut stack: Vec<Node> = vec![root];

    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if HEREDOC_PAYLOAD.contains(&kind) {
            let start = node.start_byte();
            let end = node.end_byte();
            if start < end && end <= script.len() {
                ranges.push((start, end));
            }
            // Do not walk into children — the outer body/content range is enough.
            continue;
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }

    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

/// True when the half-open byte range `[start, end)` lies entirely inside one
/// of the given (sorted) payload ranges.
pub fn range_fully_inside(start: usize, end: usize, ranges: &[(usize, usize)]) -> bool {
    if end < start {
        return false;
    }
    ranges.iter().any(|&(rs, re)| start >= rs && end <= re)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_breaks_ignored_inside_quotes() {
        let script = "echo 'a && b'; c";
        let breaks = soft_break_offsets_after_operators(script);
        // Only the real `;` (byte 13) after the quoted string, and the `&&`
        // noise must be absent. Break offset = end byte of the operator.
        assert_eq!(breaks, vec![14]);
    }

    #[test]
    fn heredoc_body_breaks_nothing() {
        let script = "cat <<EOF\nbody && junk\nEOF";
        let payload = heredoc_payload_byte_ranges(script);
        assert!(!payload.is_empty());
    }

    #[test]
    fn display_lines_preserve_multiline_command() {
        let lines = render_bash_command_display_lines("echo one && echo two", 8);
        assert!(lines.len() >= 2);
    }
}

//! Virtual terminal buffer for safely capturing and rendering PTY output in Ratatui.
//!
//! Features:
//! - Full ANSI SGR style parsing (16 colors, 256 colors, RGB truecolor, bold, dim, italic, underline, reverse)
//! - Safe escape sequence handling (cursor movements, OSC/DCS/APC strings, mode toggles);
//!   private-marker sequences (`CSI > … m`, `CSI ? … u`, `CSI … $p`) are never mistaken
//!   for SGR / cursor commands
//! - Alternate screen (`?1049h`/`?1047h`/`?47h`): full-screen programs such as `agy`
//!   are rendered on a fixed rows×cols grid with absolute cursor addressing, autowrap,
//!   scroll regions, insert/delete line/char, erase and repeat
//! - Double-width characters occupy two cells; zero-width code points are dropped so
//!   the grid always matches the cell widths Ratatui renders
//! - Handling of carriage return `\r` (overwriting lines for spinners and progress bars)
//! - Backspace `\x08`, Tab `\t`, Linefeed `\n`
//! - Support for cursor up `\x1b[1A` and clear line `\x1b[2K` for interactive CLI updates
//! - Bounded ring-buffer / scrollback capacity (up to 5,000 lines)
//! - Follow mode with auto-scroll and manual scroll navigation (Up, Down, PgUp, PgDn, Home, End)

use std::path::{Path, PathBuf};

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

/// Simple percent-decode for OSC 7 paths (e.g. `%20` -> `' '`).
fn percent_decode(input: &str) -> String {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                output.push(val);
                i += 3;
                continue;
            }
        }
        output.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

/// Marker stored in the cell to the right of a double-width character.
pub const WIDE_SPACER: char = '\u{0}';

/// Hardware cursor shapes requested by child processes via DECSCUSR (`CSI Ps SP q`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    #[default]
    Default,
    BlinkingBlock,
    SteadyBlock,
    BlinkingUnderline,
    SteadyUnderline,
    BlinkingBar,
    SteadyBar,
}

impl CursorShape {
    /// Convert this cursor shape to crossterm's cursor style.
    pub fn to_crossterm(self) -> crossterm::cursor::SetCursorStyle {
        match self {
            CursorShape::Default => crossterm::cursor::SetCursorStyle::DefaultUserShape,
            CursorShape::BlinkingBlock => crossterm::cursor::SetCursorStyle::BlinkingBlock,
            CursorShape::SteadyBlock => crossterm::cursor::SetCursorStyle::SteadyBlock,
            CursorShape::BlinkingUnderline => crossterm::cursor::SetCursorStyle::BlinkingUnderScore,
            CursorShape::SteadyUnderline => crossterm::cursor::SetCursorStyle::SteadyUnderScore,
            CursorShape::BlinkingBar => crossterm::cursor::SetCursorStyle::BlinkingBar,
            CursorShape::SteadyBar => crossterm::cursor::SetCursorStyle::SteadyBar,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledChar {
    pub c: char,
    pub style: Style,
    pub link: Option<std::sync::Arc<str>>,
}

impl StyledChar {
    pub fn new(c: char, style: Style) -> Self {
        Self { c, style, link: None }
    }

    pub fn with_link(c: char, style: Style, link: Option<std::sync::Arc<str>>) -> Self {
        Self { c, style, link }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TerminalLine {
    pub chars: Vec<StyledChar>,
}

impl TerminalLine {
    pub fn new() -> Self {
        Self { chars: Vec::new() }
    }

    pub fn from_plain_str(text: &str, style: Style) -> Self {
        let mut line = Self::new();
        let mut col = 0;
        for c in text.chars() {
            let w = c.width().unwrap_or(0);
            if w > 0 {
                line.put(col, c, style, w);
                col += w;
            }
        }
        line
    }

    pub fn set_char_with_link(
        &mut self,
        col: usize,
        c: char,
        style: Style,
        link: Option<std::sync::Arc<str>>,
    ) {
        while self.chars.len() < col {
            self.chars.push(StyledChar::new(' ', Style::default()));
        }
        let sc = StyledChar::with_link(c, style, link);
        if col < self.chars.len() {
            self.chars[col] = sc;
        } else {
            self.chars.push(sc);
        }
    }

    pub fn set_char(&mut self, col: usize, c: char, style: Style) {
        self.set_char_with_link(col, c, style, None);
    }

    /// Blank the other half of any double-width character that overlaps `from..to`.
    fn split_wide_at_edges(&mut self, from: usize, to: usize) {
        if from > 0 && self.chars.get(from).is_some_and(|sc| sc.c == WIDE_SPACER) {
            self.chars[from - 1].c = ' ';
        }
        if to > 0 && self.chars.get(to).is_some_and(|sc| sc.c == WIDE_SPACER) {
            self.chars[to].c = ' ';
        }
    }

    /// Write a character of display width `width` (1 or 2) at `col`.
    pub fn put(&mut self, col: usize, c: char, style: Style, width: usize) {
        self.put_with_link(col, c, style, width, None);
    }

    /// Write a character of display width `width` with optional hyperlink at `col`.
    pub fn put_with_link(
        &mut self,
        col: usize,
        c: char,
        style: Style,
        width: usize,
        link: Option<std::sync::Arc<str>>,
    ) {
        self.split_wide_at_edges(col, col + width);
        self.set_char_with_link(col, c, style, link.clone());
        if width == 2 {
            self.set_char_with_link(col + 1, WIDE_SPACER, style, link);
        }
    }

    /// Erase cells `from..to` with `blank`. Unstyled erasure past the end of the
    /// line simply shortens it, so plain lines never grow trailing padding.
    fn erase(&mut self, from: usize, to: usize, blank: &StyledChar) {
        self.split_wide_at_edges(from, to);
        let plain = blank.style == Style::default();
        if plain && to >= self.chars.len() {
            self.chars.truncate(from);
            self.trim_trailing_spaces();
            return;
        }
        let end = if plain { to.min(self.chars.len()) } else { to };
        for i in from..end {
            self.set_char(i, ' ', blank.style);
        }
        if plain {
            self.trim_trailing_spaces();
        }
    }

    pub fn trim_trailing_spaces(&mut self) {
        while let Some(last) = self.chars.last() {
            if last.c == ' ' && last.style == Style::default() {
                self.chars.pop();
            } else {
                break;
            }
        }
    }

    pub fn truncate(&mut self, col: usize) {
        if col < self.chars.len() {
            self.split_wide_at_edges(col, col);
            self.chars.truncate(col);
        }
        self.trim_trailing_spaces();
    }

    pub fn clear(&mut self) {
        self.chars.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn to_ratatui_line(&self) -> Line<'static> {
        chars_to_ratatui_line(&self.chars)
    }

    pub fn to_plain_string(&self) -> String {
        self.chars
            .iter()
            .filter(|sc| sc.c != WIDE_SPACER)
            .map(|sc| sc.c)
            .collect()
    }
}

pub fn chars_to_ratatui_line(chars: &[StyledChar]) -> Line<'static> {
    let mut end = chars.len();
    while end > 0 && chars[end - 1].c == ' ' && chars[end - 1].style == Style::default() {
        end -= 1;
    }
    let chars = &chars[..end];
    if chars.is_empty() {
        return Line::from("");
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut current_text = String::new();
    let mut current_style = chars[0].style;
    if chars[0].link.is_some() {
        current_style = current_style.add_modifier(Modifier::UNDERLINED);
    }

    // Wide-character spacer cells are skipped: Ratatui itself advances two
    // columns for a double-width character.
    for sc in chars.iter().filter(|sc| sc.c != WIDE_SPACER) {
        let mut cell_style = sc.style;
        if sc.link.is_some() {
            cell_style = cell_style.add_modifier(Modifier::UNDERLINED);
        }
        if cell_style == current_style {
            current_text.push(sc.c);
        } else {
            if !current_text.is_empty() {
                spans.push(Span::styled(current_text, current_style));
                current_text = String::new();
            }
            current_style = cell_style;
            current_text.push(sc.c);
        }
    }
    if !current_text.is_empty() {
        spans.push(Span::styled(current_text, current_style));
    }

    Line::from(spans)
}

/// Single search match coordinate within the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    pub line_idx: usize,
    pub start_col: usize,
    pub end_col: usize,
}

/// State of an interactive scrollback search.
#[derive(Debug, Clone, Default)]
pub struct TerminalSearch {
    pub query: String,
    pub active: bool,
    pub editing: bool,
    pub matches: Vec<SearchMatch>,
    pub current_idx: usize,
}

/// Terminal notification sent by child process via OSC 9 or OSC 777.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalNotification {
    pub title: Option<String>,
    pub body: String,
}

/// Render a slice of characters to a ratatui line with search matches highlighted.
pub fn chars_to_ratatui_line_with_search(
    chars: &[StyledChar],
    start_col: usize,
    buf_row: usize,
    search: &TerminalSearch,
) -> Line<'static> {
    if !search.active || search.query.is_empty() {
        return chars_to_ratatui_line(chars);
    }

    let mut end = chars.len();
    while end > 0 && chars[end - 1].c == ' ' && chars[end - 1].style == Style::default() {
        end -= 1;
    }
    let chars = &chars[..end];
    if chars.is_empty() {
        return Line::from("");
    }

    // Find matches on this line overlapping [start_col, start_col + chars.len())
    let row_matches: Vec<(usize, &SearchMatch)> = search
        .matches
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.line_idx == buf_row
                && m.start_col < start_col + chars.len()
                && m.end_col > start_col
        })
        .collect();

    if row_matches.is_empty() {
        return chars_to_ratatui_line(chars);
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut current_text = String::new();
    let mut current_style = Style::default();
    let mut first = true;

    for (i, sc) in chars.iter().enumerate().filter(|(_, sc)| sc.c != WIDE_SPACER) {
        let abs_col = start_col + i;
        let mut cell_style = sc.style;
        if sc.link.is_some() {
            cell_style = cell_style.add_modifier(Modifier::UNDERLINED);
        }

        for &(match_idx, m) in &row_matches {
            if abs_col >= m.start_col && abs_col < m.end_col {
                if match_idx == search.current_idx {
                    cell_style = Style::default()
                        .bg(Color::Rgb(0, 220, 255))
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD);
                } else {
                    cell_style = Style::default()
                        .bg(Color::Rgb(255, 200, 40))
                        .fg(Color::Black);
                }
                break;
            }
        }

        if first {
            current_style = cell_style;
            current_text.push(sc.c);
            first = false;
        } else if cell_style == current_style {
            current_text.push(sc.c);
        } else {
            if !current_text.is_empty() {
                spans.push(Span::styled(current_text, current_style));
                current_text = String::new();
            }
            current_style = cell_style;
            current_text.push(sc.c);
        }
    }
    if !current_text.is_empty() {
        spans.push(Span::styled(current_text, current_style));
    }

    Line::from(spans)
}

#[derive(Debug, Clone)]
pub struct ScrollInfo {
    pub total_lines: usize,
    pub viewport_height: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub follow: bool,
    pub scroll_offset: usize,
}

/// Normal-screen state preserved while a program uses the alternate screen.
#[derive(Debug, Clone)]
struct MainScreen {
    lines: Vec<TerminalLine>,
    cursor_row: usize,
    cursor_col: usize,
    saved_cursor: Option<(usize, usize)>,
    scroll_offset: usize,
    follow: bool,
}

#[derive(Debug, Clone)]
pub struct TerminalBuffer {
    pub lines: Vec<TerminalLine>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub prev_line_col: usize,
    pub cursor_visible: bool,
    pub saved_cursor: Option<(usize, usize)>,
    pub current_style: Style,
    pub pending_esc: String,
    pub max_lines: usize,

    // Scroll state
    pub scroll_offset: usize, // Lines scrolled up from the latest output
    pub follow: bool,         // Auto-follow to newest output

    /// Screen size (matches the PTY size sent to the agent).
    pub rows: usize,
    pub cols: usize,
    alt: Option<Box<MainScreen>>,
    /// DECSTBM scroll region (0-based, inclusive), alternate screen only.
    scroll_region: Option<(usize, usize)>,
    /// Autowrap is pending after writing the last column (alternate screen).
    wrap_pending: bool,
    last_printed: Option<char>,
    /// Whether bracketed paste mode is enabled by the running child process (?2004h/?2004l).
    pub bracketed_paste: bool,
    /// Hardware cursor shape set by child process (DECSCUSR).
    pub cursor_shape: CursorShape,
    /// Dynamic window/process title set by child process (OSC 0 / OSC 2).
    pub title: Option<String>,
    /// Interactive scrollback search state.
    pub search: TerminalSearch,
    /// Active hyperlink URL currently in effect for subsequent printed text (OSC 8).
    pub current_link: Option<std::sync::Arc<str>>,
    /// Current working directory reported by child process via OSC 7.
    pub cwd: Option<PathBuf>,
    /// Pending terminal desktop notifications received via OSC 9 or OSC 777.
    pub pending_notifications: Vec<TerminalNotification>,
}

impl Default for TerminalBuffer {
    fn default() -> Self {
        let mut buf = Self::new(5000);
        // The session view gives the terminal the whole screen minus the
        // header and footer rows — the same size the PTY is resized to.
        if let Ok((cols, rows)) = crossterm::terminal::size() {
            buf.resize(rows.saturating_sub(2) as usize, cols as usize);
        }
        buf
    }
}

impl TerminalBuffer {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: Vec::new(),
            cursor_row: 0,
            cursor_col: 0,
            prev_line_col: 0,
            cursor_visible: true,
            saved_cursor: None,
            current_style: Style::default(),
            pending_esc: String::new(),
            max_lines,
            scroll_offset: 0,
            follow: true,
            rows: 24,
            cols: 80,
            alt: None,
            scroll_region: None,
            wrap_pending: false,
            last_printed: None,
            bracketed_paste: false,
            cursor_shape: CursorShape::Default,
            title: None,
            search: TerminalSearch::default(),
            current_link: None,
            cwd: None,
            pending_notifications: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.alt = None;
        self.lines.clear();
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.prev_line_col = 0;
        self.cursor_visible = true;
        self.saved_cursor = None;
        self.pending_esc.clear();
        self.scroll_offset = 0;
        self.follow = true;
        self.scroll_region = None;
        self.wrap_pending = false;
        self.bracketed_paste = false;
        self.cursor_shape = CursorShape::Default;
        self.title = None;
        self.search = TerminalSearch::default();
        self.current_link = None;
        self.cwd = None;
        self.pending_notifications.clear();
    }

    /// Dynamic terminal/process title reported via OSC 0 / OSC 2, if any.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Current hardware cursor shape requested via DECSCUSR.
    pub fn cursor_shape(&self) -> CursorShape {
        self.cursor_shape
    }

    /// Current working directory reported by child process via OSC 7, if any.
    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    /// Take all pending terminal notifications, clearing the buffer's queue.
    pub fn take_notifications(&mut self) -> Vec<TerminalNotification> {
        std::mem::take(&mut self.pending_notifications)
    }

    /// Active hyperlink URL currently in effect for subsequent printed characters.
    pub fn current_link(&self) -> Option<&str> {
        self.current_link.as_deref()
    }

    /// Retrieve hyperlink URL at specific line and column, if any.
    pub fn get_link_at(&self, row: usize, col: usize) -> Option<&str> {
        self.lines.get(row).and_then(|line| {
            line.chars.get(col).and_then(|sc| sc.link.as_deref())
        })
    }

    /// Extract all distinct hyperlink URLs present in the terminal buffer in order.
    pub fn extract_hyperlinks(&self) -> Vec<String> {
        let mut links = Vec::new();
        for line in &self.lines {
            for sc in &line.chars {
                if let Some(ref l) = sc.link {
                    let s = l.to_string();
                    if !links.contains(&s) {
                        links.push(s);
                    }
                }
            }
        }
        links
    }

    /// Start an interactive search session.
    pub fn start_search(&mut self) {
        self.search.active = true;
        self.search.editing = true;
        self.follow = false;
    }

    /// Cancel search mode and clear highlighted matches.
    pub fn cancel_search(&mut self) {
        self.search.active = false;
        self.search.editing = false;
        self.search.query.clear();
        self.search.matches.clear();
        self.search.current_idx = 0;
    }

    /// Update search query and recalculate matches.
    pub fn set_search_query(&mut self, query: &str) {
        self.search.query = query.to_string();
        self.search.active = true;
        self.search.matches.clear();
        self.search.current_idx = 0;

        if query.trim().is_empty() {
            return;
        }

        let q_chars: Vec<char> = query.chars().collect();
        let q_len = q_chars.len();

        for (r_idx, line) in self.lines.iter().enumerate() {
            let mut plain_to_cell = Vec::new();
            let mut text_chars = Vec::new();
            for (cell_idx, sc) in line.chars.iter().enumerate() {
                if sc.c != WIDE_SPACER {
                    plain_to_cell.push(cell_idx);
                    text_chars.push(sc.c);
                }
            }

            if text_chars.len() >= q_len {
                for i in 0..=(text_chars.len() - q_len) {
                    let matches = (0..q_len).all(|k| {
                        text_chars[i + k]
                            .to_lowercase()
                            .eq(q_chars[k].to_lowercase())
                    });
                    if matches {
                        let start_cell = plain_to_cell[i];
                        let end_cell = if i + q_len < plain_to_cell.len() {
                            plain_to_cell[i + q_len]
                        } else {
                            line.chars.len()
                        };
                        self.search.matches.push(SearchMatch {
                            line_idx: r_idx,
                            start_col: start_cell,
                            end_col: end_cell,
                        });
                    }
                }
            }
        }

        if !self.search.matches.is_empty() {
            self.search.current_idx = self.search.matches.len().saturating_sub(1);
            self.scroll_to_match(self.search.current_idx, 24);
        }
    }

    /// Append a character to the current search query and refresh matches.
    pub fn push_search_char(&mut self, c: char) {
        self.search.query.push(c);
        let q = self.search.query.clone();
        self.set_search_query(&q);
    }

    /// Pop a character from the search query and refresh matches.
    pub fn pop_search_char(&mut self) {
        self.search.query.pop();
        let q = self.search.query.clone();
        self.set_search_query(&q);
    }

    /// Jump to the next match.
    pub fn next_search_match(&mut self, viewport_height: usize) {
        if !self.search.matches.is_empty() {
            self.search.current_idx = (self.search.current_idx + 1) % self.search.matches.len();
            self.scroll_to_match(self.search.current_idx, viewport_height);
        }
    }

    /// Jump to the previous match.
    pub fn prev_search_match(&mut self, viewport_height: usize) {
        if !self.search.matches.is_empty() {
            if self.search.current_idx == 0 {
                self.search.current_idx = self.search.matches.len() - 1;
            } else {
                self.search.current_idx -= 1;
            }
            self.scroll_to_match(self.search.current_idx, viewport_height);
        }
    }

    /// Scroll such that target line is centered in viewport.
    pub fn scroll_to_line(&mut self, target_buf_row: usize, viewport_height: usize) {
        let total = self.lines.len();
        if total == 0 {
            return;
        }
        let vh = viewport_height.max(1);
        let target_start = target_buf_row.saturating_sub(vh / 2);
        let max_scroll = total.saturating_sub(vh);
        self.scroll_offset = total.saturating_sub(target_start + vh).min(max_scroll);
        self.follow = false;
    }

    /// Scroll such that the indexed match is visible in the viewport.
    pub fn scroll_to_match(&mut self, match_idx: usize, viewport_height: usize) {
        if let Some(m) = self.search.matches.get(match_idx).copied() {
            self.scroll_to_line(m.line_idx, viewport_height);
        }
    }

    /// Whether bracketed paste mode is enabled by the running child process.
    pub fn bracketed_paste_enabled(&self) -> bool {
        self.bracketed_paste
    }

    /// Whether a full-screen program currently owns the (alternate) screen.
    pub fn in_alt_screen(&self) -> bool {
        self.alt.is_some()
    }

    /// Whether the slash-command completion popup is currently open and visible on the screen.
    pub fn is_completion_open(&self) -> bool {
        if self.in_alt_screen() {
            return false;
        }
        let start = self.cursor_row.saturating_sub(2);
        let end = (self.cursor_row + 20).min(self.lines.len());
        for r in start..end {
            let s = self.lines[r].to_plain_string();
            if s.contains("esc to cancel")
                || (s.contains("Navigate") && (s.contains("Select") || s.contains("Complete")))
            {
                return true;
            }
        }
        false
    }

    /// Resize the screen grid (call with the same size sent to the PTY).
    pub fn resize(&mut self, rows: usize, cols: usize) {
        let (rows, cols) = (rows.max(1), cols.max(1));
        if (rows, cols) == (self.rows, self.cols) {
            return;
        }
        self.rows = rows;
        self.cols = cols;
        self.scroll_region = None;
        self.wrap_pending = false;
        if self.in_alt_screen() {
            self.lines.resize_with(rows, TerminalLine::new);
            for line in &mut self.lines {
                line.truncate(cols);
            }
            self.cursor_row = self.cursor_row.min(rows - 1);
            self.cursor_col = self.cursor_col.min(cols - 1);
        }
    }

    fn ensure_cursor(&mut self) {
        while self.lines.len() <= self.cursor_row {
            self.lines.push(TerminalLine::new());
        }
    }

    fn enforce_capacity(&mut self) {
        if self.in_alt_screen() {
            return;
        }
        if self.lines.len() > self.max_lines {
            let overflow = self.lines.len() - self.max_lines;
            self.lines.drain(0..overflow);
            self.cursor_row = self.cursor_row.saturating_sub(overflow);
            if let Some((saved_r, saved_c)) = self.saved_cursor {
                self.saved_cursor = Some((saved_r.saturating_sub(overflow), saved_c));
            }
        }
    }

    /// First buffer row of the visible screen on the normal screen.
    fn screen_top(&self) -> usize {
        self.lines
            .len()
            .max(self.cursor_row + 1)
            .saturating_sub(self.rows)
    }

    /// Active scroll region (alternate screen), inclusive.
    fn region(&self) -> (usize, usize) {
        match self.scroll_region {
            Some((t, b)) if t < b && b < self.rows => (t, b),
            _ => (0, self.rows - 1),
        }
    }

    /// Blank cell used for erasure: keeps the current background (BCE).
    fn blank(&self) -> StyledChar {
        let style = match self.current_style.bg {
            Some(bg) if bg != Color::Reset => Style::default().bg(bg),
            _ => Style::default(),
        };
        StyledChar::new(' ', style)
    }

    fn scroll_region_up(&mut self, n: usize) {
        let (t, b) = self.region();
        for _ in 0..n.min(b - t + 1) {
            self.lines.remove(t);
            self.lines.insert(b, TerminalLine::new());
        }
    }

    fn scroll_region_down(&mut self, n: usize) {
        let (t, b) = self.region();
        for _ in 0..n.min(b - t + 1) {
            self.lines.remove(b);
            self.lines.insert(t, TerminalLine::new());
        }
    }

    /// Line feed. On the alternate screen this is a real terminal LF (column
    /// kept, scrolling at the bottom of the region); on the normal screen the
    /// historical behaviour (implicit CR, remembered column) is kept.
    fn linefeed(&mut self) {
        self.wrap_pending = false;
        if self.in_alt_screen() {
            let (_, b) = self.region();
            if self.cursor_row == b {
                self.scroll_region_up(1);
            } else if self.cursor_row + 1 < self.rows {
                self.cursor_row += 1;
            }
            return;
        }
        self.cursor_row += 1;
        if self.cursor_col > 0 {
            self.prev_line_col = self.cursor_col;
        }
        self.cursor_col = 0;
        self.enforce_capacity();
    }

    fn print(&mut self, c: char) {
        // Zero-width code points (combining marks, variation selectors, ZWJ)
        // are dropped so every stored cell matches Ratatui's width model.
        let w = c.width().unwrap_or(0);
        if w == 0 {
            return;
        }
        // Printable character resets pending line-relative tracking
        self.prev_line_col = 0;
        self.last_printed = Some(c);
        let style = self.current_style;
        if self.in_alt_screen() {
            if self.wrap_pending {
                self.cursor_col = 0;
                self.linefeed();
            }
            if self.cursor_col + w > self.cols {
                if w > self.cols {
                    return;
                }
                self.cursor_col = 0;
                self.linefeed();
            }
            let (row, col) = (self.cursor_row, self.cursor_col);
            self.lines[row].put_with_link(col, c, style, w, self.current_link.clone());
            if col + w >= self.cols {
                self.cursor_col = self.cols - 1;
                self.wrap_pending = true;
            } else {
                self.cursor_col = col + w;
            }
        } else {
            self.ensure_cursor();
            self.lines[self.cursor_row].put_with_link(
                self.cursor_col,
                c,
                style,
                w,
                self.current_link.clone(),
            );
            self.cursor_col += w;
        }
    }

    fn enter_alt_screen(&mut self) {
        if self.in_alt_screen() {
            return;
        }
        self.alt = Some(Box::new(MainScreen {
            lines: std::mem::take(&mut self.lines),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            saved_cursor: self.saved_cursor.take(),
            scroll_offset: self.scroll_offset,
            follow: self.follow,
        }));
        self.lines = vec![TerminalLine::new(); self.rows];
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.prev_line_col = 0;
        self.scroll_offset = 0;
        self.follow = true;
        self.scroll_region = None;
        self.wrap_pending = false;
    }

    fn leave_alt_screen(&mut self) {
        if let Some(main) = self.alt.take() {
            self.lines = main.lines;
            self.cursor_row = main.cursor_row;
            self.cursor_col = main.cursor_col;
            self.saved_cursor = main.saved_cursor;
            self.scroll_offset = main.scroll_offset;
            self.follow = main.follow;
            self.prev_line_col = 0;
            self.scroll_region = None;
            self.wrap_pending = false;
        }
    }

    /// Push raw PTY text chunk into the virtual terminal buffer.
    pub fn push_str(&mut self, text: &str) {
        let input = if self.pending_esc.is_empty() {
            text.to_string()
        } else {
            let mut combined = std::mem::take(&mut self.pending_esc);
            combined.push_str(text);
            combined
        };

        let mut chars = input.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '\x1b' => {
                    // Escape sequence encountered
                    let mut esc_seq = String::from("\x1b");
                    let mut complete = false;

                    match chars.peek() {
                        Some(&'[') => {
                            // CSI sequence: \x1b[ ... final_byte
                            esc_seq.push(chars.next().unwrap());
                            while let Some(&next_c) = chars.peek() {
                                esc_seq.push(chars.next().unwrap());
                                if (0x40..=0x7E).contains(&(next_c as u32)) {
                                    complete = true;
                                    break;
                                }
                            }
                        }
                        Some(&']') | Some(&'P') | Some(&'_') | Some(&'^') | Some(&'X') => {
                            // OSC / DCS / APC / PM / SOS string: terminated by BEL or ST (\x1b\)
                            esc_seq.push(chars.next().unwrap());
                            while let Some(&next_c) = chars.peek() {
                                esc_seq.push(chars.next().unwrap());
                                if next_c == '\x07' {
                                    complete = true;
                                    break;
                                }
                                if esc_seq.ends_with("\x1b\\") {
                                    complete = true;
                                    break;
                                }
                            }
                        }
                        Some(&'(') | Some(&')') | Some(&'*') | Some(&'+') | Some(&'#')
                        | Some(&' ') => {
                            // Character set selection: \x1b(B etc.
                            esc_seq.push(chars.next().unwrap());
                            if let Some(&_next_c) = chars.peek() {
                                esc_seq.push(chars.next().unwrap());
                                complete = true;
                            }
                        }
                        Some(_) => {
                            // 2-char escape (ESC 7, ESC 8, ESC M, ESC D, ESC E, ESC c, ESC =, ...)
                            esc_seq.push(chars.next().unwrap());
                            complete = true;
                        }
                        None => {
                            // Chunk ended right after \x1b
                            complete = false;
                        }
                    }

                    if !complete {
                        // Sequence not completed within this chunk, buffer it for next chunk
                        self.pending_esc = esc_seq;
                        break;
                    } else {
                        self.handle_escape_sequence(&esc_seq);
                    }
                }
                '\r' => {
                    // Carriage return: reset cursor to column 0 on the current row
                    self.cursor_col = 0;
                    self.prev_line_col = 0;
                    self.wrap_pending = false;
                }
                '\n' => self.linefeed(),
                '\x0b' | '\x0c' if self.in_alt_screen() => self.linefeed(),
                '\t' => {
                    self.prev_line_col = 0;
                    self.wrap_pending = false;
                    // Advance to next 8-column tab stop
                    let tab_width = 8;
                    self.cursor_col = ((self.cursor_col / tab_width) + 1) * tab_width;
                    if self.in_alt_screen() {
                        self.cursor_col = self.cursor_col.min(self.cols - 1);
                    }
                }
                '\x08' | '\x7f' => {
                    // Backspace / Delete
                    let old_col = if self.cursor_col == 0 && self.prev_line_col > 0 {
                        self.prev_line_col
                    } else {
                        self.cursor_col
                    };
                    self.cursor_col = old_col.saturating_sub(1);
                    self.prev_line_col = 0;
                    self.wrap_pending = false;
                    if !self.in_alt_screen() && self.cursor_row < self.lines.len() {
                        let line = &mut self.lines[self.cursor_row];
                        line.trim_trailing_spaces();
                        if old_col >= line.chars.len() && self.cursor_col < line.chars.len() {
                            line.chars.truncate(self.cursor_col);
                            line.trim_trailing_spaces();
                        }
                    }
                }
                c if c.is_control() => {
                    // Ignore other control characters (e.g. \x00, \x07, \x0c)
                }
                c => self.print(c),
            }
        }
    }

    /// Add a high-level system event or notification line directly to the terminal.
    /// While a full-screen program is active the line goes to the normal screen.
    pub fn push_system_line(&mut self, text: &str, style: Style) {
        if let Some(main) = self.alt.as_mut() {
            main.lines.push(TerminalLine::from_plain_str(text, style));
            main.cursor_row = main.lines.len();
            main.cursor_col = 0;
            return;
        }
        self.ensure_cursor();
        if !self.lines[self.cursor_row].is_empty() {
            self.cursor_row += 1;
            self.ensure_cursor();
        }
        self.lines[self.cursor_row] = TerminalLine::from_plain_str(text, style);
        self.cursor_row += 1;
        self.cursor_col = 0;
        self.ensure_cursor();
        self.enforce_capacity();
    }

    /// Interpret escape sequences safely without letting raw sequences escape.
    fn handle_escape_sequence(&mut self, seq: &str) {
        let alt = self.in_alt_screen();
        match seq {
            "\x1bM" => {
                // Reverse Index: move cursor up 1 line (scrolls at the top of the region)
                self.wrap_pending = false;
                self.prev_line_col = 0;
                if alt && self.cursor_row == self.region().0 {
                    self.scroll_region_down(1);
                } else {
                    self.cursor_row = self.cursor_row.saturating_sub(1);
                    self.ensure_cursor();
                }
                return;
            }
            "\x1bD" => {
                // Index: line feed without carriage return
                let col = self.cursor_col;
                self.linefeed();
                self.cursor_col = col;
                return;
            }
            "\x1bE" => {
                // Next Line
                self.cursor_col = 0;
                self.linefeed();
                self.cursor_col = 0;
                return;
            }
            "\x1b7" => {
                // Save cursor position
                self.saved_cursor = Some((self.cursor_row, self.cursor_col));
                return;
            }
            "\x1b8" => {
                // Restore cursor position
                self.restore_cursor();
                return;
            }
            "\x1bc" => {
                // Full reset
                self.clear();
                self.current_style = Style::default();
                return;
            }
            _ => {}
        }

        if seq.starts_with("\x1b]") {
            // OSC sequence (Operating System Command)
            let osc_body = &seq[2..];
            let osc_content = if let Some(stripped) = osc_body.strip_suffix("\x1b\\") {
                stripped
            } else if let Some(stripped) = osc_body.strip_suffix('\x07') {
                stripped
            } else {
                osc_body
            };

            if let Some((cmd, payload)) = osc_content.split_once(';') {
                match cmd {
                    "0" | "2" => {
                        // OSC 0: Set window icon name and title; OSC 2: Set window title
                        if payload.is_empty() {
                            self.title = None;
                        } else {
                            self.title = Some(payload.to_string());
                        }
                    }
                    "7" => {
                        // OSC 7: Current Working Directory (`\x1b]7;file://hostname/path\x07` or `\x1b]7;/path\x07`)
                        if payload.is_empty() {
                            self.cwd = None;
                        } else {
                            let raw_path = payload.strip_prefix("file://").unwrap_or(payload);
                            let path_part = if !raw_path.starts_with('/') {
                                if let Some(slash_idx) = raw_path.find('/') {
                                    &raw_path[slash_idx..]
                                } else {
                                    raw_path
                                }
                            } else {
                                raw_path
                            };
                            let decoded = percent_decode(path_part);
                            if !decoded.is_empty() {
                                self.cwd = Some(PathBuf::from(decoded));
                            }
                        }
                    }
                    "8" => {
                        // OSC 8: Hyperlink sequence (`\x1b]8;params;url\x1b\` or `\x1b]8;;\x1b\`)
                        let (_params, url) = payload.split_once(';').unwrap_or(("", payload));
                        if url.is_empty() {
                            self.current_link = None;
                        } else {
                            self.current_link = Some(std::sync::Arc::from(url));
                        }
                    }
                    "9" => {
                        // OSC 9: iTerm2 style desktop notification (`\x1b]9;message\x07`)
                        if !payload.is_empty() {
                            self.pending_notifications.push(TerminalNotification {
                                title: None,
                                body: payload.to_string(),
                            });
                        }
                    }
                    "777" => {
                        // OSC 777: rxvt/Kitty style notification (`\x1b]777;notify;title;body\x07`)
                        if let Some(rest) = payload.strip_prefix("notify;") {
                            let (title, body) = rest.split_once(';').unwrap_or((rest, ""));
                            self.pending_notifications.push(TerminalNotification {
                                title: if title.is_empty() {
                                    None
                                } else {
                                    Some(title.to_string())
                                },
                                body: body.to_string(),
                            });
                        }
                    }
                    _ => {}
                }
            }
            return;
        }

        if !seq.starts_with("\x1b[") {
            // Other DCS / APC or 2-char escape safely discarded
            return;
        }

        let body = &seq[2..];
        if body.is_empty() {
            return;
        }

        let last_char = body.chars().last().unwrap();
        let params_str = &body[..body.len() - 1];

        // Handle DECSCUSR: Set Cursor Style (`CSI Ps SP q` or `CSI Ps q`)
        if last_char == 'q'
            && (params_str.is_empty()
                || params_str
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == ' '))
        {
            let mode = params_str.trim().parse::<u32>().unwrap_or(0);
            self.cursor_shape = match mode {
                1 => CursorShape::BlinkingBlock,
                2 => CursorShape::SteadyBlock,
                3 => CursorShape::BlinkingUnderline,
                4 => CursorShape::SteadyUnderline,
                5 => CursorShape::BlinkingBar,
                6 => CursorShape::SteadyBar,
                _ => CursorShape::Default,
            };
            return;
        }

        // Sequences with intermediate bytes (DECRQM `$p`, ...) are
        // queries / mode reports, not screen operations.
        if params_str.chars().any(|c| ('\x20'..='\x2f').contains(&c)) {
            return;
        }

        // Private-marker sequences (`?`, `>`, `<`, `=`): only DEC private modes
        // matter. Crucially `CSI > 4 ; 2 m` (modifyOtherKeys) is not SGR and
        // `CSI > 1 u` / `CSI ? u` (kitty keyboard) do not restore the cursor.
        if let Some(first) = params_str
            .chars()
            .next()
            .filter(|c| ('<'..='?').contains(c))
        {
            if first == '?' && (last_char == 'h' || last_char == 'l') {
                let set = last_char == 'h';
                for mode in params_str[1..].split(';') {
                    match mode {
                        "25" => self.cursor_visible = set,
                        "2004" => self.bracketed_paste = set,
                        "1049" | "1047" | "47" => {
                            if set {
                                if mode == "1049" {
                                    self.saved_cursor = Some((self.cursor_row, self.cursor_col));
                                }
                                self.enter_alt_screen();
                            } else {
                                self.leave_alt_screen();
                            }
                        }
                        _ => {}
                    }
                }
            }
            return;
        }

        if last_char == 'm' {
            // SGR - Select Graphic Rendition (Colors and styling)
            self.parse_sgr(&params_str.replace(':', ";"));
            return;
        }

        self.wrap_pending = false;
        let params: Vec<usize> = params_str
            .split(';')
            .map(|s| s.parse().unwrap_or(0))
            .collect();
        let raw = |i: usize| params.get(i).copied().unwrap_or(0);
        let n = |i: usize| params.get(i).copied().filter(|&v| v > 0).unwrap_or(1);
        let rows = self.rows;

        if alt {
            self.handle_alt_csi(last_char, params_str, raw(0), n(0), n(1), params.len());
            return;
        }

        match last_char {
            'K' => {
                let mode = params_str.parse::<u32>().unwrap_or(0);
                if self.cursor_row < self.lines.len() {
                    match mode {
                        0 => {
                            self.lines[self.cursor_row].truncate(self.cursor_col);
                        }
                        1 => {
                            for i in 0..self.cursor_col.min(self.lines[self.cursor_row].chars.len())
                            {
                                self.lines[self.cursor_row].chars[i] =
                                    StyledChar::new(' ', self.current_style);
                            }
                        }
                        2 => {
                            self.lines[self.cursor_row].clear();
                        }
                        _ => {}
                    }
                }
            }
            'J' => {
                self.prev_line_col = 0;
                let mode = params_str.parse::<u32>().unwrap_or(0);
                match mode {
                    0 => {
                        if self.cursor_row < self.lines.len() {
                            self.lines[self.cursor_row].truncate(self.cursor_col);
                            self.lines.truncate(self.cursor_row + 1);
                        }
                    }
                    1 => {
                        for r in 0..self.cursor_row.min(self.lines.len()) {
                            self.lines[r].clear();
                        }
                        if self.cursor_row < self.lines.len() {
                            for c in 0..self.cursor_col.min(self.lines[self.cursor_row].chars.len())
                            {
                                self.lines[self.cursor_row].chars[c] =
                                    StyledChar::new(' ', self.current_style);
                            }
                        }
                    }
                    2 | 3 => {
                        self.lines.clear();
                        self.cursor_row = 0;
                        self.cursor_col = 0;
                    }
                    _ => {}
                }
            }
            'A' | 'F' => {
                // Cursor Up: \x1b[<n>A (critical for multi-line spinners and interactive CLIs)
                self.cursor_row = self.cursor_row.saturating_sub(n(0));
                self.prev_line_col = 0;
                if last_char == 'F' {
                    self.cursor_col = 0;
                }
            }
            'B' | 'E' => {
                // Cursor Down: \x1b[<n>B
                self.cursor_row += n(0);
                self.prev_line_col = 0;
                if last_char == 'E' {
                    self.cursor_col = 0;
                }
                self.enforce_capacity();
            }
            'C' | 'a' => {
                // Cursor Forward
                let count = n(0);
                if self.cursor_col == 0 && self.prev_line_col > 0 {
                    self.cursor_col = self.prev_line_col + count;
                } else {
                    self.cursor_col += count;
                }
                self.prev_line_col = 0;
            }
            'D' => {
                // Cursor Backward
                let count = n(0);
                if self.cursor_col == 0 && self.prev_line_col > 0 {
                    self.cursor_col = self.prev_line_col.saturating_sub(count);
                } else {
                    self.cursor_col = self.cursor_col.saturating_sub(count);
                }
                self.prev_line_col = 0;
            }
            'G' | '`' => {
                // Cursor Horizontal Absolute: \x1b[<col>G
                self.cursor_col = n(0) - 1;
                self.prev_line_col = 0;
            }
            'd' => {
                // Line Position Absolute: \x1b[<row>d (relative to the visible screen)
                self.cursor_row = self.screen_top() + (n(0) - 1).min(rows - 1);
                self.prev_line_col = 0;
                self.ensure_cursor();
            }
            'H' | 'f' => {
                // Cursor Position: \x1b[<row>;<col>H (row relative to the visible screen)
                self.prev_line_col = 0;
                self.cursor_row = self.screen_top() + (n(0) - 1).min(rows - 1);
                self.cursor_col = n(1) - 1;
            }
            'L' => {
                self.ensure_cursor();
                for _ in 0..n(0) {
                    self.lines.insert(self.cursor_row, TerminalLine::new());
                }
                self.enforce_capacity();
            }
            'M' => {
                for _ in 0..n(0) {
                    if self.cursor_row < self.lines.len() {
                        self.lines.remove(self.cursor_row);
                    }
                }
            }
            'P' | '@' | 'X' | 'b' => self.line_edit(last_char, n(0)),
            's' if params_str.is_empty() => {
                // Save Cursor: \x1b[s
                self.saved_cursor = Some((self.cursor_row, self.cursor_col));
            }
            'u' if params_str.is_empty() => {
                // Restore Cursor: \x1b[u
                self.restore_cursor();
            }
            _ => {
                // Modes, queries and others are safely swallowed
            }
        }
    }

    fn restore_cursor(&mut self) {
        if let Some((r, c)) = self.saved_cursor {
            self.cursor_row = r;
            self.cursor_col = c;
            self.prev_line_col = 0;
            self.wrap_pending = false;
            if self.in_alt_screen() {
                self.cursor_row = r.min(self.rows - 1);
                self.cursor_col = c.min(self.cols - 1);
            }
            self.ensure_cursor();
        }
    }

    /// Character-level edits shared by both screens: DCH, ICH, ECH, REP.
    fn line_edit(&mut self, op: char, count: usize) {
        if op == 'b' {
            if let Some(c) = self.last_printed {
                for _ in 0..count.min(self.rows * self.cols) {
                    self.print(c);
                }
            }
            return;
        }
        self.ensure_cursor();
        let (col, cols, alt, blank) = (
            self.cursor_col,
            self.cols,
            self.in_alt_screen(),
            self.blank(),
        );
        let line = &mut self.lines[self.cursor_row];
        match op {
            'P' => {
                if col < line.chars.len() {
                    line.split_wide_at_edges(col, (col + count).min(line.chars.len()));
                    let end = (col + count).min(line.chars.len());
                    line.chars.drain(col..end);
                }
            }
            '@' => {
                if col < line.chars.len() {
                    line.split_wide_at_edges(col, col);
                    for _ in 0..count.min(cols) {
                        line.chars.insert(col, blank.clone());
                    }
                    if alt {
                        line.truncate(cols);
                    }
                }
            }
            'X' => {
                let end = if alt {
                    (col + count).min(cols)
                } else {
                    col + count
                };
                line.erase(col, end, &blank);
            }
            _ => {}
        }
    }

    /// CSI handling for the alternate screen: a fixed rows×cols grid.
    fn handle_alt_csi(
        &mut self,
        op: char,
        params_str: &str,
        p0: usize,
        n0: usize,
        n1: usize,
        nparams: usize,
    ) {
        let (rows, cols) = (self.rows, self.cols);
        let (top, bottom) = self.region();
        let in_region = self.cursor_row >= top && self.cursor_row <= bottom;
        let blank = self.blank();
        match op {
            'K' => {
                let (row, col) = (self.cursor_row, self.cursor_col);
                let (from, to) = match p0 {
                    0 => (col, cols),
                    1 => (0, col + 1),
                    2 => (0, cols),
                    _ => return,
                };
                self.lines[row].erase(from, to, &blank);
            }
            'J' => {
                let (row, col) = (self.cursor_row, self.cursor_col);
                let (lines_from, lines_to) = match p0 {
                    0 => {
                        self.lines[row].erase(col, cols, &blank);
                        (row + 1, rows)
                    }
                    1 => {
                        self.lines[row].erase(0, col + 1, &blank);
                        (0, row)
                    }
                    2 | 3 => (0, rows),
                    _ => return,
                };
                for r in lines_from..lines_to {
                    self.lines[r].erase(0, cols, &blank);
                }
            }
            'A' | 'F' => {
                let floor = if in_region { top } else { 0 };
                self.cursor_row = self
                    .cursor_row
                    .saturating_sub(n0)
                    .max(floor.min(self.cursor_row));
                if op == 'F' {
                    self.cursor_col = 0;
                }
            }
            'B' | 'E' => {
                let ceil = if in_region { bottom } else { rows - 1 };
                self.cursor_row = (self.cursor_row + n0).min(ceil.max(self.cursor_row));
                if op == 'E' {
                    self.cursor_col = 0;
                }
            }
            'C' | 'a' => self.cursor_col = (self.cursor_col + n0).min(cols - 1),
            'D' => self.cursor_col = self.cursor_col.saturating_sub(n0),
            'G' | '`' => self.cursor_col = (n0 - 1).min(cols - 1),
            'd' => self.cursor_row = (n0 - 1).min(rows - 1),
            'e' => self.cursor_row = (self.cursor_row + n0).min(rows - 1),
            'H' | 'f' => {
                self.cursor_row = (n0 - 1).min(rows - 1);
                self.cursor_col = (n1 - 1).min(cols - 1);
            }
            'L' | 'M' if in_region => {
                for _ in 0..n0.min(bottom - self.cursor_row + 1) {
                    if op == 'L' {
                        self.lines.remove(bottom);
                        self.lines.insert(self.cursor_row, TerminalLine::new());
                    } else {
                        self.lines.remove(self.cursor_row);
                        self.lines.insert(bottom, TerminalLine::new());
                    }
                }
                self.cursor_col = 0;
            }
            'S' => self.scroll_region_up(n0),
            'T' if nparams <= 1 => self.scroll_region_down(n0),
            'r' => {
                let t = n0 - 1;
                let b = if nparams > 1 && n1 > 0 {
                    n1 - 1
                } else {
                    rows - 1
                };
                self.scroll_region = (t < b && b < rows).then_some((t, b));
                self.cursor_row = 0;
                self.cursor_col = 0;
            }
            'P' | '@' | 'X' | 'b' => self.line_edit(op, n0),
            's' if params_str.is_empty() => {
                self.saved_cursor = Some((self.cursor_row, self.cursor_col))
            }
            'u' if params_str.is_empty() => self.restore_cursor(),
            _ => {}
        }
    }

    /// Parse ANSI SGR parameters (e.g. "1;32;40") and apply to current_style.
    fn parse_sgr(&mut self, params_str: &str) {
        if params_str.is_empty() {
            self.current_style = Style::default();
            return;
        }

        let parts: Vec<u32> = params_str
            .split(';')
            .filter_map(|s| s.parse::<u32>().ok())
            .collect();

        if parts.is_empty() {
            self.current_style = Style::default();
            return;
        }

        let mut idx = 0;
        while idx < parts.len() {
            match parts[idx] {
                0 => {
                    self.current_style = Style::default();
                    idx += 1;
                }
                1 => {
                    self.current_style = self.current_style.add_modifier(Modifier::BOLD);
                    idx += 1;
                }
                2 => {
                    self.current_style = self.current_style.add_modifier(Modifier::DIM);
                    idx += 1;
                }
                3 => {
                    self.current_style = self.current_style.add_modifier(Modifier::ITALIC);
                    idx += 1;
                }
                4 => {
                    self.current_style = self.current_style.add_modifier(Modifier::UNDERLINED);
                    idx += 1;
                }
                7 => {
                    self.current_style = self.current_style.add_modifier(Modifier::REVERSED);
                    idx += 1;
                }
                9 => {
                    self.current_style = self.current_style.add_modifier(Modifier::CROSSED_OUT);
                    idx += 1;
                }
                22 => {
                    self.current_style = self
                        .current_style
                        .remove_modifier(Modifier::BOLD | Modifier::DIM);
                    idx += 1;
                }
                23 => {
                    self.current_style = self.current_style.remove_modifier(Modifier::ITALIC);
                    idx += 1;
                }
                24 => {
                    self.current_style = self.current_style.remove_modifier(Modifier::UNDERLINED);
                    idx += 1;
                }
                27 => {
                    self.current_style = self.current_style.remove_modifier(Modifier::REVERSED);
                    idx += 1;
                }
                29 => {
                    self.current_style = self.current_style.remove_modifier(Modifier::CROSSED_OUT);
                    idx += 1;
                }
                // Standard Foreground Colors
                30 => {
                    self.current_style = self.current_style.fg(Color::Black);
                    idx += 1;
                }
                31 => {
                    self.current_style = self.current_style.fg(Color::Red);
                    idx += 1;
                }
                32 => {
                    self.current_style = self.current_style.fg(Color::Green);
                    idx += 1;
                }
                33 => {
                    self.current_style = self.current_style.fg(Color::Yellow);
                    idx += 1;
                }
                34 => {
                    self.current_style = self.current_style.fg(Color::Blue);
                    idx += 1;
                }
                35 => {
                    self.current_style = self.current_style.fg(Color::Magenta);
                    idx += 1;
                }
                36 => {
                    self.current_style = self.current_style.fg(Color::Cyan);
                    idx += 1;
                }
                37 => {
                    self.current_style = self.current_style.fg(Color::Gray);
                    idx += 1;
                }
                // Extended Foreground Color
                38 => {
                    if idx + 2 < parts.len() && parts[idx + 1] == 5 {
                        let color_val = parts[idx + 2] as u8;
                        self.current_style = self.current_style.fg(Color::Indexed(color_val));
                        idx += 3;
                    } else if idx + 4 < parts.len() && parts[idx + 1] == 2 {
                        let r = parts[idx + 2] as u8;
                        let g = parts[idx + 3] as u8;
                        let b = parts[idx + 4] as u8;
                        self.current_style = self.current_style.fg(Color::Rgb(r, g, b));
                        idx += 5;
                    } else {
                        idx += 1;
                    }
                }
                39 => {
                    self.current_style = self.current_style.fg(Color::Reset);
                    idx += 1;
                }
                // Standard Background Colors
                40 => {
                    self.current_style = self.current_style.bg(Color::Black);
                    idx += 1;
                }
                41 => {
                    self.current_style = self.current_style.bg(Color::Red);
                    idx += 1;
                }
                42 => {
                    self.current_style = self.current_style.bg(Color::Green);
                    idx += 1;
                }
                43 => {
                    self.current_style = self.current_style.bg(Color::Yellow);
                    idx += 1;
                }
                44 => {
                    self.current_style = self.current_style.bg(Color::Blue);
                    idx += 1;
                }
                45 => {
                    self.current_style = self.current_style.bg(Color::Magenta);
                    idx += 1;
                }
                46 => {
                    self.current_style = self.current_style.bg(Color::Cyan);
                    idx += 1;
                }
                47 => {
                    self.current_style = self.current_style.bg(Color::Gray);
                    idx += 1;
                }
                // Extended Background Color
                48 => {
                    if idx + 2 < parts.len() && parts[idx + 1] == 5 {
                        let color_val = parts[idx + 2] as u8;
                        self.current_style = self.current_style.bg(Color::Indexed(color_val));
                        idx += 3;
                    } else if idx + 4 < parts.len() && parts[idx + 1] == 2 {
                        let r = parts[idx + 2] as u8;
                        let g = parts[idx + 3] as u8;
                        let b = parts[idx + 4] as u8;
                        self.current_style = self.current_style.bg(Color::Rgb(r, g, b));
                        idx += 5;
                    } else {
                        idx += 1;
                    }
                }
                49 => {
                    self.current_style = self.current_style.bg(Color::Reset);
                    idx += 1;
                }
                // Bright Foreground Colors
                90 => {
                    self.current_style = self.current_style.fg(Color::DarkGray);
                    idx += 1;
                }
                91 => {
                    self.current_style = self.current_style.fg(Color::LightRed);
                    idx += 1;
                }
                92 => {
                    self.current_style = self.current_style.fg(Color::LightGreen);
                    idx += 1;
                }
                93 => {
                    self.current_style = self.current_style.fg(Color::LightYellow);
                    idx += 1;
                }
                94 => {
                    self.current_style = self.current_style.fg(Color::LightBlue);
                    idx += 1;
                }
                95 => {
                    self.current_style = self.current_style.fg(Color::LightMagenta);
                    idx += 1;
                }
                96 => {
                    self.current_style = self.current_style.fg(Color::LightCyan);
                    idx += 1;
                }
                97 => {
                    self.current_style = self.current_style.fg(Color::White);
                    idx += 1;
                }
                // Bright Background Colors
                100 => {
                    self.current_style = self.current_style.bg(Color::DarkGray);
                    idx += 1;
                }
                101 => {
                    self.current_style = self.current_style.bg(Color::LightRed);
                    idx += 1;
                }
                102 => {
                    self.current_style = self.current_style.bg(Color::LightGreen);
                    idx += 1;
                }
                103 => {
                    self.current_style = self.current_style.bg(Color::LightYellow);
                    idx += 1;
                }
                104 => {
                    self.current_style = self.current_style.bg(Color::LightBlue);
                    idx += 1;
                }
                105 => {
                    self.current_style = self.current_style.bg(Color::LightMagenta);
                    idx += 1;
                }
                106 => {
                    self.current_style = self.current_style.bg(Color::LightCyan);
                    idx += 1;
                }
                107 => {
                    self.current_style = self.current_style.bg(Color::White);
                    idx += 1;
                }
                _ => {
                    idx += 1;
                }
            }
        }
    }

    /// Total number of lines currently buffered.
    pub fn total_lines(&self) -> usize {
        self.lines.len()
    }

    /// User scrolled up by `delta` lines. The alternate screen has no
    /// scrollback (the program owns the whole screen), so this is a no-op there.
    pub fn scroll_up(&mut self, delta: usize) {
        if self.in_alt_screen() {
            return;
        }
        let total = self.total_lines();
        self.follow = false;
        self.scroll_offset = (self.scroll_offset + delta).min(total.saturating_sub(1));
    }

    /// User scrolled down by `delta` lines.
    pub fn scroll_down(&mut self, delta: usize) {
        if self.scroll_offset <= delta {
            self.scroll_offset = 0;
            self.follow = true;
        } else {
            self.scroll_offset -= delta;
        }
    }

    /// User scrolled to the very top (Home).
    pub fn scroll_to_top(&mut self) {
        let total = self.total_lines();
        if total > 0 && !self.in_alt_screen() {
            self.follow = false;
            self.scroll_offset = total.saturating_sub(1);
        }
    }

    /// User scrolled to the bottom (End).
    pub fn scroll_to_bottom(&mut self) {
        self.follow = true;
        self.scroll_offset = 0;
    }

    /// Get visible lines for a viewport of given height.
    pub fn get_visible_lines(&self, viewport_height: usize) -> (Vec<Line<'static>>, ScrollInfo) {
        let total = self.total_lines();
        if total == 0 {
            return (
                Vec::new(),
                ScrollInfo {
                    total_lines: 0,
                    viewport_height,
                    start_line: 0,
                    end_line: 0,
                    follow: self.follow,
                    scroll_offset: 0,
                },
            );
        }

        let effective_offset = if self.follow {
            0
        } else {
            self.scroll_offset.min(total.saturating_sub(1))
        };

        let end_line = total.saturating_sub(effective_offset);
        let start_line = end_line.saturating_sub(viewport_height);

        let mut result = Vec::with_capacity(end_line.saturating_sub(start_line));

        for i in start_line..end_line {
            result.push(chars_to_ratatui_line_with_search(
                &self.lines[i].chars,
                0,
                i,
                &self.search,
            ));
        }

        (
            result,
            ScrollInfo {
                total_lines: total,
                viewport_height,
                start_line,
                end_line,
                follow: self.follow,
                scroll_offset: effective_offset,
            },
        )
    }

    /// Alternate screen: the grid is shown exactly as the program drew it —
    /// no re-wrapping, no scrollback — with the cursor at its grid position.
    fn alt_screen_view(
        &self,
        viewport_height: usize,
        viewport_width: usize,
    ) -> (Vec<Line<'static>>, ScrollInfo, Option<(u16, u16)>) {
        let shown = self.lines.len().min(viewport_height);
        let mut result: Vec<Line<'static>> = self.lines[..shown]
            .iter()
            .map(TerminalLine::to_ratatui_line)
            .collect();
        result.resize(viewport_height, Line::from(""));
        let cursor = (self.cursor_visible
            && self.cursor_row < viewport_height
            && self.cursor_col < viewport_width)
            .then_some((self.cursor_col as u16, self.cursor_row as u16));
        (
            result,
            ScrollInfo {
                total_lines: self.lines.len(),
                viewport_height,
                start_line: 0,
                end_line: shown,
                follow: true,
                scroll_offset: 0,
            },
            cursor,
        )
    }

    /// Get visible lines wrapped to viewport_width, padded to viewport_height,
    /// along with scroll info and visual cursor position.
    pub fn get_visible_lines_wrapped(
        &self,
        viewport_height: usize,
        viewport_width: usize,
    ) -> (Vec<Line<'static>>, ScrollInfo, Option<(u16, u16)>) {
        if viewport_height == 0 {
            return (
                Vec::new(),
                ScrollInfo {
                    total_lines: 0,
                    viewport_height: 0,
                    start_line: 0,
                    end_line: 0,
                    follow: self.follow,
                    scroll_offset: 0,
                },
                None,
            );
        }

        if self.in_alt_screen() {
            return self.alt_screen_view(viewport_height, viewport_width);
        }

        if viewport_width == 0 {
            let (lines, info) = self.get_visible_lines(viewport_height);
            let cursor = if self.follow
                && self.cursor_visible
                && self.cursor_row >= info.start_line
                && self.cursor_row < info.end_line
            {
                let rel_row = (self.cursor_row - info.start_line) as u16;
                let rel_col = self.cursor_col as u16;
                Some((rel_col, rel_row))
            } else {
                None
            };
            return (lines, info, cursor);
        }

        struct VisualLineEntry {
            line: Line<'static>,
            buf_row: usize,
            start_col: usize,
            end_col: usize,
        }

        let mut visual_lines: Vec<VisualLineEntry> = Vec::new();

        for (r_idx, line) in self.lines.iter().enumerate() {
            let effective_len = {
                let mut e = line.chars.len();
                while e > 0
                    && line.chars[e - 1].c == ' '
                    && line.chars[e - 1].style == Style::default()
                {
                    e -= 1;
                }
                e
            };
            if effective_len == 0 {
                visual_lines.push(VisualLineEntry {
                    line: Line::from(""),
                    buf_row: r_idx,
                    start_col: 0,
                    end_col: 0,
                });
            } else if effective_len <= viewport_width {
                visual_lines.push(VisualLineEntry {
                    line: chars_to_ratatui_line_with_search(
                        &line.chars[..effective_len],
                        0,
                        r_idx,
                        &self.search,
                    ),
                    buf_row: r_idx,
                    start_col: 0,
                    end_col: effective_len,
                });
            } else {
                let mut start = 0;
                while start < effective_len {
                    let mut end = (start + viewport_width).min(effective_len);
                    // Never split a double-width character across two rows.
                    if end < effective_len && line.chars[end].c == WIDE_SPACER && end > start + 1 {
                        end -= 1;
                    }
                    visual_lines.push(VisualLineEntry {
                        line: chars_to_ratatui_line_with_search(
                            &line.chars[start..end],
                            start,
                            r_idx,
                            &self.search,
                        ),
                        buf_row: r_idx,
                        start_col: start,
                        end_col: end,
                    });
                    start = end;
                }
            }
        }

        // If cursor is on a line beyond existing lines (e.g. fresh empty line awaiting input)
        if self.cursor_row >= self.lines.len() {
            visual_lines.push(VisualLineEntry {
                line: Line::from(""),
                buf_row: self.cursor_row,
                start_col: 0,
                end_col: 0,
            });
        }

        // Find visual position of the cursor
        let mut cursor_visual_row = None;
        let mut cursor_visual_col = 0;

        for (v_idx, ventry) in visual_lines.iter().enumerate() {
            if ventry.buf_row == self.cursor_row {
                let is_last_chunk = v_idx + 1 == visual_lines.len()
                    || visual_lines[v_idx + 1].buf_row != self.cursor_row;
                if self.cursor_col >= ventry.start_col
                    && (self.cursor_col < ventry.end_col || is_last_chunk)
                {
                    cursor_visual_row = Some(v_idx);
                    cursor_visual_col = self.cursor_col.saturating_sub(ventry.start_col);
                    break;
                }
            }
        }

        let total_visual = visual_lines.len();
        let effective_offset = if self.follow {
            0
        } else {
            self.scroll_offset.min(total_visual.saturating_sub(1))
        };

        let end_idx = total_visual.saturating_sub(effective_offset);
        let start_idx = end_idx.saturating_sub(viewport_height);

        let mut result_lines = Vec::with_capacity(viewport_height);
        for i in start_idx..end_idx {
            result_lines.push(visual_lines[i].line.clone());
        }

        // Fill remaining viewport space with blank lines to ensure clean redraws
        while result_lines.len() < viewport_height {
            result_lines.push(Line::from(""));
        }

        let cursor_pos = if let Some(v_row) = cursor_visual_row {
            if self.cursor_visible && v_row >= start_idx && v_row < end_idx {
                let rel_row = (v_row - start_idx) as u16;
                let rel_col =
                    (cursor_visual_col as u16).min(viewport_width.saturating_sub(1) as u16);
                Some((rel_col, rel_row))
            } else {
                None
            }
        } else {
            None
        };

        (
            result_lines,
            ScrollInfo {
                total_lines: total_visual,
                viewport_height,
                start_line: start_idx,
                end_line: end_idx,
                follow: self.follow,
                scroll_offset: effective_offset,
            },
            cursor_pos,
        )
    }

    /// Returns plain text representation of the last N lines.
    pub fn get_recent_plain_lines(&self, n: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let total = self.total_lines();
        let start = total.saturating_sub(n);
        for i in start..total {
            lines.push(self.lines[i].to_plain_string());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_and_newlines() {
        let mut buf = TerminalBuffer::new(100);
        buf.push_str("Hello world\nSecond line\nThird line");
        assert_eq!(buf.lines.len(), 3);
        assert_eq!(buf.lines[0].to_plain_string(), "Hello world");
        assert_eq!(buf.lines[1].to_plain_string(), "Second line");
        assert_eq!(buf.lines[2].to_plain_string(), "Third line");
    }

    #[test]
    fn test_carriage_return_overwrite() {
        let mut buf = TerminalBuffer::new(100);
        buf.push_str("Progress: 10%\rProgress: 50%\rProgress: 100%\nDone!");
        assert_eq!(buf.lines.len(), 2);
        assert_eq!(buf.lines[0].to_plain_string(), "Progress: 100%");
        assert_eq!(buf.lines[1].to_plain_string(), "Done!");
    }

    #[test]
    fn test_multiline_cursor_up_spinner() {
        let mut buf = TerminalBuffer::new(100);
        // Initial 2 newlines emitted by interactive CLI
        buf.push_str("⡿  Running c\n\n\x1b[4D");
        assert_eq!(buf.total_lines(), 1);
        assert_eq!(buf.lines[0].to_plain_string(), "⡿  Running c");

        // Spinner updates frame with cursor up 2 lines and overwrite:
        buf.push_str("\x1b[?25l\r\x1b[2A⢿  Running co\n\n\x1b[5D\x1b[?25h");
        assert_eq!(buf.total_lines(), 1);
        assert_eq!(buf.lines[0].to_plain_string(), "⢿  Running co");

        // Another frame:
        buf.push_str("\x1b[?25l\r\x1b[2A⣻  Running com\n\n\x1b[6D\x1b[?25h");
        assert_eq!(buf.total_lines(), 1);
        assert_eq!(buf.lines[0].to_plain_string(), "⣻  Running com");
    }

    #[test]
    fn test_ansi_colors_and_styles() {
        let mut buf = TerminalBuffer::new(100);
        buf.push_str("\x1b[1;32mSUCCESS\x1b[0m normal text\n");
        assert_eq!(buf.lines.len(), 1);
        let rat_line = buf.lines[0].to_ratatui_line();
        assert_eq!(rat_line.spans.len(), 2);
        assert_eq!(rat_line.spans[0].content, "SUCCESS");
        assert_eq!(rat_line.spans[0].style.fg, Some(Color::Green));
        assert!(rat_line.spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert_eq!(rat_line.spans[1].content, " normal text");
    }

    #[test]
    fn test_strip_cursor_and_mode_escapes() {
        let mut buf = TerminalBuffer::new(100);
        // Feed various escape codes: hide cursor, alternate screen, OSC title, move cursor up
        buf.push_str("\x1b[?25l\x1b[?1049h\x1b]0;Antigravity\x07Clean output\x1b[?25h\n");
        assert!(buf.in_alt_screen());
        assert_eq!(buf.lines[0].to_plain_string(), "Clean output");
        assert!(buf
            .lines
            .iter()
            .all(|l| !l.to_plain_string().contains('\x1b')));
    }

    fn alt(rows: usize, cols: usize) -> TerminalBuffer {
        let mut buf = TerminalBuffer::new(100);
        buf.resize(rows, cols);
        buf.push_str("\x1b[?1049h");
        buf
    }

    fn row(buf: &TerminalBuffer, r: usize) -> String {
        buf.lines[r].to_plain_string()
    }

    #[test]
    fn alt_screen_absolute_positioning_overwrites_instead_of_appending() {
        let mut buf = alt(5, 20);
        buf.push_str("\x1b[H\x1b[2Jfirst frame\r\nline two");
        buf.push_str("\x1b[Hsecond\x1b[K\r\n\x1b[K");
        assert_eq!(row(&buf, 0), "second");
        assert_eq!(row(&buf, 1), "");
        assert_eq!(buf.lines.len(), 5);
        buf.push_str("\x1b[3;5Hxy");
        assert_eq!(row(&buf, 2), "    xy");
        assert_eq!((buf.cursor_row, buf.cursor_col), (2, 6));
    }

    #[test]
    fn alt_screen_lf_keeps_column_and_leave_restores_main() {
        let mut buf = TerminalBuffer::new(100);
        buf.push_str("shell$ ");
        buf.push_str("\x1b[?1049h\x1b[3Gab\ncd");
        assert_eq!(row(&buf, 0), "  ab");
        assert_eq!(row(&buf, 1), "    cd");
        buf.push_str("\x1b[?1049l");
        assert!(!buf.in_alt_screen());
        assert_eq!(buf.lines.len(), 1);
        assert_eq!(row(&buf, 0), "shell$ ");
        assert_eq!(buf.cursor_col, 7);
    }

    #[test]
    fn alt_screen_autowrap_and_scroll() {
        let mut buf = alt(3, 4);
        buf.push_str("abcdefgh");
        assert_eq!(row(&buf, 0), "abcd");
        assert_eq!(row(&buf, 1), "efgh");
        assert!(buf.wrap_pending);
        buf.push_str("ij\r\nkl");
        assert_eq!(row(&buf, 0), "efgh");
        assert_eq!(row(&buf, 1), "ij");
        assert_eq!(row(&buf, 2), "kl");
    }

    #[test]
    fn alt_screen_scroll_region_and_line_ops() {
        let mut buf = alt(5, 10);
        buf.push_str("\x1b[1;1H0\x1b[2;1H1\x1b[3;1H2\x1b[4;1H3\x1b[5;1H4");
        buf.push_str("\x1b[2;4r\x1b[4;1H\n");
        let rows: Vec<String> = (0..5).map(|r| row(&buf, r)).collect();
        assert_eq!(rows, ["0", "2", "3", "", "4"]);
        buf.push_str("\x1b[r\x1b[2;1H\x1b[L");
        let rows: Vec<String> = (0..5).map(|r| row(&buf, r)).collect();
        assert_eq!(rows, ["0", "", "2", "3", ""]);
        buf.push_str("\x1b[1;1Habcdef\x1b[1;2H\x1b[2P\x1b[1;1H\x1b[2@");
        assert_eq!(row(&buf, 0), "  adef");
        buf.push_str("\x1b[1;3H\x1b[2X");
        assert_eq!(row(&buf, 0), "    ef");
        buf.push_str("\x1b[2;1H=\x1b[4b");
        assert_eq!(row(&buf, 1), "=====");
    }

    #[test]
    fn private_marker_sequences_do_not_change_style_or_cursor() {
        let mut buf = TerminalBuffer::new(100);
        buf.push_str("ab\x1b[s");
        buf.push_str("\x1b[>4;2m\x1b[>1u\x1b[?u\x1b[?2026$p\x1b[2 qcd");
        assert_eq!(
            buf.current_style,
            Style::default(),
            "CSI > 4;2 m must not set DIM"
        );
        assert_eq!(row(&buf, 0), "abcd");
        assert_eq!(buf.cursor_col, 4);
    }

    #[test]
    fn dcs_and_apc_strings_are_swallowed() {
        let mut buf = TerminalBuffer::new(100);
        buf.push_str("a\x1bP+q544e\x1b\\b\x1b_Gf=100;AAAA\x1b\\c");
        assert_eq!(row(&buf, 0), "abc");
    }

    #[test]
    fn wide_and_zero_width_characters_keep_the_grid_aligned() {
        let mut buf = alt(2, 10);
        buf.push_str("界x\u{0301}⚠\u{FE0F}|");
        assert_eq!(row(&buf, 0), "界x⚠|");
        assert_eq!(buf.lines[0].chars.len(), 5);
        assert_eq!(buf.cursor_col, 5);
        // Overwriting the right half of a wide char blanks its left half.
        buf.push_str("\x1b[1;2HZ");
        assert_eq!(row(&buf, 0), " Zx⚠|");
        let line = buf.lines[0].to_ratatui_line();
        assert_eq!(line.width(), 5);
    }

    #[test]
    fn block_and_braille_glyphs_are_single_cells_with_colors() {
        let mut buf = alt(2, 20);
        buf.push_str("\x1b[38;2;1;2;3;48;2;4;5;6m▀\x1b[m▄█▌▐░▒▓▸⣾");
        assert_eq!(row(&buf, 0), "▀▄█▌▐░▒▓▸⣾");
        assert_eq!(buf.cursor_col, 10);
        let c = &buf.lines[0].chars[0];
        assert_eq!(c.style.fg, Some(Color::Rgb(1, 2, 3)));
        assert_eq!(c.style.bg, Some(Color::Rgb(4, 5, 6)));
        assert_eq!(buf.lines[0].chars[1].style, Style::default());
    }

    #[test]
    fn resize_alt_screen_clamps_grid_and_cursor() {
        let mut buf = alt(10, 40);
        buf.push_str("\x1b[10;40Hz");
        buf.resize(5, 20);
        assert_eq!(buf.lines.len(), 5);
        assert!(buf.cursor_row < 5 && buf.cursor_col < 20);
        let (lines, _, _) = buf.get_visible_lines_wrapped(5, 20);
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn real_agy_startup_capture_renders_logo_intact() {
        let raw = include_bytes!("../tests/fixtures/agy_startup_100x30.raw");
        let text = String::from_utf8(raw.to_vec()).unwrap();
        // Feed in small pieces to exercise escape sequences split across chunks.
        let mut buf = TerminalBuffer::new(5000);
        buf.resize(30, 100);
        let chars: Vec<char> = text.chars().collect();
        for piece in chars.chunks(7) {
            buf.push_str(&piece.iter().collect::<String>());
        }
        assert!(buf.in_alt_screen());
        assert_eq!(buf.lines.len(), 30);
        let screen: Vec<String> = buf.lines.iter().map(|l| l.to_plain_string()).collect();
        // The trust prompt was drawn from the top (\x1b[H) over the logo frame.
        assert_eq!(screen[0], "Accessing workspace:");
        assert!(screen
            .iter()
            .any(|l| l.contains("> Yes, I trust this folder")));
        assert!(screen
            .iter()
            .all(|l| !l.contains('\u{FFFD}') && !l.contains('\x1b')));
        assert_eq!(buf.current_style, Style::default());
        assert!(buf.lines.iter().all(|l| l.chars.len() <= 100));

        // First frame alone: the logo rows are at their exact columns.
        let first = text.split("\x1b[H\x1b[33;1m").next().unwrap();
        let mut logo = TerminalBuffer::new(5000);
        logo.resize(30, 100);
        logo.push_str(first);
        let l: Vec<String> = logo.lines.iter().map(|l| l.to_plain_string()).collect();
        assert_eq!(l[1], "     ▄▀▀▄");
        assert_eq!(l[2], "    ▀▀▀▀▀▀");
        assert_eq!(l[3], "   ▀▀▀▀▀▀▀▀");
        assert_eq!(l[4], "  ▄▀▀    ▀▀▄");
        assert_eq!(l[5], " ▄▀▀      ▀▀▄");
        assert!(l[7].contains("Welcome to the Antigravity CLI"));
        assert!(l[9].contains("⣾  Signing in..."));
        // Half blocks keep both colours (top = fg, bottom = bg).
        let cell = &logo.lines[1].chars[6];
        assert_eq!(cell.c, '▀');
        assert_eq!(cell.style.fg, Some(Color::Rgb(242, 146, 46)));
        assert_eq!(cell.style.bg, Some(Color::Rgb(246, 145, 46)));
        assert!(!cell.style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn test_clear_line_escape() {
        let mut buf = TerminalBuffer::new(100);
        buf.push_str("Old bad text\x1b[2K\rNew clean text\n");
        assert_eq!(buf.lines.len(), 1);
        assert_eq!(buf.lines[0].to_plain_string(), "New clean text");
    }

    #[test]
    fn test_cursor_up_erase_line() {
        let mut buf = TerminalBuffer::new(100);
        buf.push_str("Thinking...\n\x1b[1A\x1b[2KCompleted thinking!\n");
        assert_eq!(buf.lines.len(), 1);
        assert_eq!(buf.lines[0].to_plain_string(), "Completed thinking!");
    }

    #[test]
    fn test_scrolling_and_follow_mode() {
        let mut buf = TerminalBuffer::new(100);
        for i in 1..=20 {
            buf.push_str(&format!("Line {}\n", i));
        }
        assert!(buf.follow);
        assert_eq!(buf.scroll_offset, 0);

        let (visible, info) = buf.get_visible_lines(5);
        assert_eq!(visible.len(), 5);
        assert_eq!(info.start_line, 15);
        assert_eq!(info.end_line, 20);

        // Scroll up
        buf.scroll_up(3);
        assert!(!buf.follow);
        assert_eq!(buf.scroll_offset, 3);
        let (_visible_scrolled, info_scrolled) = buf.get_visible_lines(5);
        assert_eq!(info_scrolled.start_line, 12);
        assert_eq!(info_scrolled.end_line, 17);

        // Scroll back to bottom
        buf.scroll_to_bottom();
        assert!(buf.follow);
        assert_eq!(buf.scroll_offset, 0);
    }

    #[test]
    fn test_wrapped_lines_and_cursor_visibility() {
        let mut buf = TerminalBuffer::new(100);
        // Push a line of 50 characters: "0123456789" repeated 5 times
        let long_text = "0123456789".repeat(5);
        buf.push_str(&long_text);
        assert_eq!(buf.cursor_col, 50);

        // Viewport with width 20 and height 10:
        // 50 chars wraps into 3 lines: 20 chars, 20 chars, 10 chars
        let (lines, info, cursor) = buf.get_visible_lines_wrapped(10, 20);
        assert_eq!(info.total_lines, 3);
        assert_eq!(lines.len(), 10); // padded to viewport height
        assert!(cursor.is_some());
        let (col, row) = cursor.unwrap();
        // Cursor at col 50: on 3rd visual line (row 2), col 10
        assert_eq!(row, 2);
        assert_eq!(col, 10);

        // Test cursor hide mode (\x1b[?25l)
        buf.push_str("\x1b[?25l");
        assert!(!buf.cursor_visible);
        let (_, _, cursor_hidden) = buf.get_visible_lines_wrapped(10, 20);
        assert!(cursor_hidden.is_none());

        // Test cursor show mode (\x1b[?25h)
        buf.push_str("\x1b[?25h");
        assert!(buf.cursor_visible);
        let (_, _, cursor_shown) = buf.get_visible_lines_wrapped(10, 20);
        assert!(cursor_shown.is_some());
    }

    #[test]
    fn test_relative_cursor_movement_after_newline() {
        let mut buf = TerminalBuffer::new(100);
        // Emulate AGY banner positioning:
        // Line 1: cursor moved to col 5, prints 4 half-blocks (cols 5..8, cursor ends at col 9)
        // Line 2: newline, then cursor back 5 -> col 4, prints 6 half-blocks (cols 4..9)
        // Line 3: newline, then cursor back 7 -> col 3, prints 8 half-blocks (cols 3..10)
        buf.push_str("\n\x1b[5C▄▀▀▄\n\x1b[5D▀▀▀▀▀▀\n\x1b[7D▀▀▀▀▀▀▀▀\r\n");
        assert_eq!(buf.lines.len(), 4);
        assert_eq!(buf.lines[1].to_plain_string(), "     ▄▀▀▄");
        assert_eq!(buf.lines[2].to_plain_string(), "    ▀▀▀▀▀▀");
        assert_eq!(buf.lines[3].to_plain_string(), "   ▀▀▀▀▀▀▀▀");
    }

    #[test]
    fn test_osc_window_title_tracking() {
        let mut buf = TerminalBuffer::new(100);
        assert_eq!(buf.title(), None);

        // OSC 0 with BEL terminator
        buf.push_str("\x1b]0;Antigravity Session\x07");
        assert_eq!(buf.title(), Some("Antigravity Session"));

        // OSC 2 with ST (\x1b\) terminator
        buf.push_str("\x1b]2;Vim: main.rs\x1b\\");
        assert_eq!(buf.title(), Some("Vim: main.rs"));

        // Reset title with empty payload
        buf.push_str("\x1b]0;\x07");
        assert_eq!(buf.title(), None);

        // Setting title again and then clearing buffer resets title
        buf.push_str("\x1b]2;Bash Terminal\x07");
        assert_eq!(buf.title(), Some("Bash Terminal"));
        buf.clear();
        assert_eq!(buf.title(), None);
    }

    #[test]
    fn test_decscusr_cursor_shapes() {
        let mut buf = TerminalBuffer::new(100);
        assert_eq!(buf.cursor_shape(), CursorShape::Default);

        buf.push_str("\x1b[1 q");
        assert_eq!(buf.cursor_shape(), CursorShape::BlinkingBlock);

        buf.push_str("\x1b[2 q");
        assert_eq!(buf.cursor_shape(), CursorShape::SteadyBlock);

        buf.push_str("\x1b[3 q");
        assert_eq!(buf.cursor_shape(), CursorShape::BlinkingUnderline);

        buf.push_str("\x1b[4 q");
        assert_eq!(buf.cursor_shape(), CursorShape::SteadyUnderline);

        buf.push_str("\x1b[5 q");
        assert_eq!(buf.cursor_shape(), CursorShape::BlinkingBar);

        buf.push_str("\x1b[6 q");
        assert_eq!(buf.cursor_shape(), CursorShape::SteadyBar);

        buf.push_str("\x1b[0 q");
        assert_eq!(buf.cursor_shape(), CursorShape::Default);

        // Test clear resets cursor shape
        buf.push_str("\x1b[5 q");
        assert_eq!(buf.cursor_shape(), CursorShape::BlinkingBar);
        buf.clear();
        assert_eq!(buf.cursor_shape(), CursorShape::Default);
    }

    #[test]
    fn test_scrollback_search_matching_and_navigation() {
        let mut buf = TerminalBuffer::new(100);
        buf.push_str("Line 1: Error found in connection\n");
        buf.push_str("Line 2: Warning: retrying request\n");
        buf.push_str("Line 3: Second Error encountered\n");
        buf.push_str("Line 4: All systems operational\n");

        assert!(!buf.search.active);
        buf.start_search();
        assert!(buf.search.active);
        assert!(buf.search.editing);
        assert!(!buf.follow);

        // Case-insensitive search for "error"
        buf.set_search_query("error");
        assert_eq!(buf.search.matches.len(), 2);
        assert_eq!(buf.search.matches[0].line_idx, 0);
        assert_eq!(buf.search.matches[1].line_idx, 2);
        // By default focuses on most recent match (match 1 on line 2)
        assert_eq!(buf.search.current_idx, 1);

        // Next match wraps to 0
        buf.next_search_match(10);
        assert_eq!(buf.search.current_idx, 0);

        // Prev match wraps back to 1
        buf.prev_search_match(10);
        assert_eq!(buf.search.current_idx, 1);

        // Test push and pop char
        buf.cancel_search();
        assert!(!buf.search.active);
        assert!(buf.search.matches.is_empty());

        buf.push_search_char('w');
        buf.push_search_char('a');
        buf.push_search_char('r');
        buf.push_search_char('n');
        assert_eq!(buf.search.query, "warn");
        assert_eq!(buf.search.matches.len(), 1);
        assert_eq!(buf.search.matches[0].line_idx, 1);

        buf.pop_search_char();
        assert_eq!(buf.search.query, "war");
        assert_eq!(buf.search.matches.len(), 1);
    }

    #[test]
    fn test_scrollback_search_highlighting_in_visible_lines() {
        let mut buf = TerminalBuffer::new(100);
        buf.push_str("Server started on 127.0.0.1:8080 successfully\n");

        buf.set_search_query("127.0.0.1");
        assert_eq!(buf.search.matches.len(), 1);

        let (lines, _info) = buf.get_visible_lines(10);
        assert!(!lines.is_empty());
        let server_line = &lines[0];

        // Verify that a span with "127.0.0.1" is styled with match highlight background
        let match_span = server_line
            .spans
            .iter()
            .find(|span| span.content == "127.0.0.1");
        assert!(match_span.is_some(), "Match span '127.0.0.1' not found in spans: {:?}", server_line.spans);
        let span = match_span.unwrap();
        assert_eq!(span.style.bg, Some(Color::Rgb(0, 220, 255))); // Focused match style
    }

    #[test]
    fn test_osc8_hyperlink_parsing_and_extraction() {
        let mut buf = TerminalBuffer::new(100);

        // Print text with OSC 8 hyperlink (ST terminator)
        buf.push_str("\x1b]8;id=link1;https://github.com/agentcontrol\x1b\\AgentControll\x1b]8;;\x1b\\ is awesome!\n");

        // Plain string should contain only display text
        assert_eq!(buf.lines[0].to_plain_string(), "AgentControll is awesome!");

        // Verify link attached to characters 0..13
        assert_eq!(
            buf.get_link_at(0, 0),
            Some("https://github.com/agentcontrol")
        );
        assert_eq!(
            buf.get_link_at(0, 12),
            Some("https://github.com/agentcontrol")
        );
        assert_eq!(buf.get_link_at(0, 13), None);

        // Verify hyperlink list extraction
        let links = buf.extract_hyperlinks();
        assert_eq!(links, vec!["https://github.com/agentcontrol"]);

        // Verify Ratatui rendering has underline modifier on linked span
        let (lines, _) = buf.get_visible_lines(10);
        let line = &lines[0];
        let link_span = line
            .spans
            .iter()
            .find(|s| s.content == "AgentControll")
            .expect("Span for 'AgentControll' must exist");
        assert!(
            link_span.style.add_modifier.contains(Modifier::UNDERLINED),
            "Hyperlinked span must have underline modifier"
        );

        let unlinked_span = line
            .spans
            .iter()
            .find(|s| s.content == " is awesome!")
            .expect("Span for ' is awesome!' must exist");
        assert!(
            !unlinked_span.style.add_modifier.contains(Modifier::UNDERLINED),
            "Unlinked span must not have underline modifier"
        );

        // Also test OSC 8 with BEL terminator
        buf.push_str("\x1b]8;;https://docs.agentcontrol.dev\x07Documentation\x1b]8;;\x07\n");
        assert_eq!(
            buf.get_link_at(1, 0),
            Some("https://docs.agentcontrol.dev")
        );
        let links_total = buf.extract_hyperlinks();
        assert_eq!(
            links_total,
            vec![
                "https://github.com/agentcontrol",
                "https://docs.agentcontrol.dev"
            ]
        );

        // Buffer clear resets hyperlinks
        buf.clear();
        assert!(buf.extract_hyperlinks().is_empty());
        assert_eq!(buf.current_link(), None);
    }

    #[test]
    fn test_osc7_cwd_tracking() {
        let mut buf = TerminalBuffer::new(100);
        assert_eq!(buf.cwd(), None);

        // OSC 7 with hostname and BEL terminator
        buf.push_str("\x1b]7;file://localhost/home/user/workspace\x07");
        assert_eq!(buf.cwd(), Some(Path::new("/home/user/workspace")));

        // OSC 7 with percent-encoding and ST terminator
        buf.push_str("\x1b]7;file:///home/user/my%20project/sub%2Fdir\x1b\\");
        assert_eq!(buf.cwd(), Some(Path::new("/home/user/my project/sub/dir")));

        // OSC 7 with empty payload resets CWD
        buf.push_str("\x1b]7;\x07");
        assert_eq!(buf.cwd(), None);

        // Buffer clear resets CWD
        buf.push_str("\x1b]7;file:///tmp\x07");
        assert_eq!(buf.cwd(), Some(Path::new("/tmp")));
        buf.clear();
        assert_eq!(buf.cwd(), None);
    }

    #[test]
    fn test_osc9_and_osc777_notifications() {
        let mut buf = TerminalBuffer::new(100);

        // OSC 9 notification
        buf.push_str("\x1b]9;Build finished successfully!\x07");
        let notifs = buf.take_notifications();
        assert_eq!(notifs.len(), 1);
        assert_eq!(
            notifs[0],
            TerminalNotification {
                title: None,
                body: "Build finished successfully!".to_string(),
            }
        );
        // After take_notifications, pending queue is empty
        assert!(buf.take_notifications().is_empty());

        // OSC 777 notification with title and body
        buf.push_str("\x1b]777;notify;Cargo;All 42 tests passed\x1b\\");
        // OSC 777 notification without title
        buf.push_str("\x1b]777;notify;;Task completed\x07");

        let notifs2 = buf.take_notifications();
        assert_eq!(notifs2.len(), 2);
        assert_eq!(
            notifs2[0],
            TerminalNotification {
                title: Some("Cargo".to_string()),
                body: "All 42 tests passed".to_string(),
            }
        );
        assert_eq!(
            notifs2[1],
            TerminalNotification {
                title: None,
                body: "Task completed".to_string(),
            }
        );
    }
}

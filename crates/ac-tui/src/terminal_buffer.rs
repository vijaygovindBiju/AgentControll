//! Virtual terminal line buffer for safely capturing and rendering PTY output in Ratatui.
//!
//! Features:
//! - Full ANSI SGR style parsing (16 colors, 256 colors, RGB truecolor, bold, dim, italic, underline, reverse)
//! - Safe escape sequence stripping (cursor movements, alternate screen, OSC titles, mode toggles)
//! - Handling of carriage return `\r` (overwriting lines for spinners and progress bars)
//! - Backspace `\x08`, Tab `\t`, Linefeed `\n`
//! - Support for cursor up `\x1b[1A` and clear line `\x1b[2K` for interactive CLI updates
//! - Bounded ring-buffer / scrollback capacity (up to 5,000 lines)
//! - Follow mode with auto-scroll and manual scroll navigation (Up, Down, PgUp, PgDn, Home, End)

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledChar {
    pub c: char,
    pub style: Style,
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
        Self {
            chars: text
                .chars()
                .map(|c| StyledChar { c, style })
                .collect(),
        }
    }

    pub fn set_char(&mut self, col: usize, c: char, style: Style) {
        while self.chars.len() < col {
            self.chars.push(StyledChar {
                c: ' ',
                style: Style::default(),
            });
        }
        if col < self.chars.len() {
            self.chars[col] = StyledChar { c, style };
        } else {
            self.chars.push(StyledChar { c, style });
        }
    }

    pub fn truncate(&mut self, col: usize) {
        if col < self.chars.len() {
            self.chars.truncate(col);
        }
    }

    pub fn clear(&mut self) {
        self.chars.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn to_ratatui_line(&self) -> Line<'static> {
        if self.chars.is_empty() {
            return Line::from("");
        }

        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut current_text = String::new();
        let mut current_style = self.chars[0].style;

        for sc in &self.chars {
            if sc.style == current_style {
                current_text.push(sc.c);
            } else {
                if !current_text.is_empty() {
                    spans.push(Span::styled(current_text, current_style));
                    current_text = String::new();
                }
                current_style = sc.style;
                current_text.push(sc.c);
            }
        }
        if !current_text.is_empty() {
            spans.push(Span::styled(current_text, current_style));
        }

        Line::from(spans)
    }

    pub fn to_plain_string(&self) -> String {
        self.chars.iter().map(|sc| sc.c).collect()
    }
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

#[derive(Debug, Clone)]
pub struct TerminalBuffer {
    pub lines: Vec<TerminalLine>,
    pub current_line: TerminalLine,
    pub cursor_col: usize,
    pub current_style: Style,
    pub pending_esc: String,
    pub max_lines: usize,

    // Scroll state
    pub scroll_offset: usize, // Lines scrolled up from the latest output
    pub follow: bool,         // Auto-follow to newest output
}

impl Default for TerminalBuffer {
    fn default() -> Self {
        Self::new(5000)
    }
}

impl TerminalBuffer {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: Vec::new(),
            current_line: TerminalLine::new(),
            cursor_col: 0,
            current_style: Style::default(),
            pending_esc: String::new(),
            max_lines,
            scroll_offset: 0,
            follow: true,
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
                        Some(&']') => {
                            // OSC sequence: \x1b] ... (\x07 or \x1b\)
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
                        Some(&'(') | Some(&')') | Some(&'*') | Some(&'+') => {
                            // Character set selection: \x1b(B etc.
                            esc_seq.push(chars.next().unwrap());
                            if let Some(&_next_c) = chars.peek() {
                                esc_seq.push(chars.next().unwrap());
                                complete = true;
                            }
                        }
                        Some(&c2) if c2 == '=' || c2 == '>' || c2 == 'M' || c2 == '7' || c2 == '8' || c2 == 'c' => {
                            esc_seq.push(chars.next().unwrap());
                            complete = true;
                        }
                        Some(_) => {
                            // Unrecognized 2-char escape
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
                    // Carriage return: reset cursor to column 0 without newline
                    self.cursor_col = 0;
                }
                '\n' => {
                    // Linefeed: commit current line and advance
                    self.commit_current_line();
                }
                '\t' => {
                    // Advance to next 8-column tab stop
                    let tab_width = 8;
                    self.cursor_col = ((self.cursor_col / tab_width) + 1) * tab_width;
                }
                '\x08' => {
                    // Backspace
                    self.cursor_col = self.cursor_col.saturating_sub(1);
                }
                c if c.is_control() => {
                    // Ignore other control characters (e.g. \x00, \x07, \x0c)
                }
                c => {
                    // Printable character
                    self.current_line.set_char(self.cursor_col, c, self.current_style);
                    self.cursor_col += 1;
                }
            }
        }
    }

    /// Add a high-level system event or notification line directly to the terminal.
    pub fn push_system_line(&mut self, text: &str, style: Style) {
        if !self.current_line.is_empty() {
            self.commit_current_line();
        }
        self.lines.push(TerminalLine::from_plain_str(text, style));
        self.enforce_capacity();
    }

    fn commit_current_line(&mut self) {
        let finished_line = std::mem::take(&mut self.current_line);
        self.lines.push(finished_line);
        self.cursor_col = 0;
        self.enforce_capacity();
    }

    fn enforce_capacity(&mut self) {
        if self.lines.len() > self.max_lines {
            let overflow = self.lines.len() - self.max_lines;
            self.lines.drain(0..overflow);
        }
    }

    /// Interpret escape sequences safely without letting raw sequences escape.
    fn handle_escape_sequence(&mut self, seq: &str) {
        if !seq.starts_with("\x1b[") {
            // OSC or 2-char escape: safely discarded
            return;
        }

        let body = &seq[2..];
        if body.is_empty() {
            return;
        }

        let last_char = body.chars().last().unwrap();
        let params_str = &body[..body.len() - 1];

        match last_char {
            'm' => {
                // SGR - Select Graphic Rendition (Colors and styling)
                self.parse_sgr(params_str);
            }
            'K' => {
                // Erase in Line
                let mode = params_str.parse::<u32>().unwrap_or(0);
                match mode {
                    0 => {
                        // Clear from cursor to end of line
                        self.current_line.truncate(self.cursor_col);
                    }
                    1 => {
                        // Clear from beginning to cursor
                        for i in 0..self.cursor_col.min(self.current_line.chars.len()) {
                            self.current_line.chars[i] = StyledChar {
                                c: ' ',
                                style: self.current_style,
                            };
                        }
                    }
                    2 => {
                        // Clear entire line
                        self.current_line.clear();
                        self.cursor_col = 0;
                    }
                    _ => {}
                }
            }
            'J' => {
                // Erase in Display (Clear screen)
                let mode = params_str.parse::<u32>().unwrap_or(0);
                if mode == 2 || mode == 3 {
                    self.current_line.clear();
                    self.cursor_col = 0;
                }
            }
            'A' => {
                // Cursor Up: \x1b[<n>A
                let count = params_str.parse::<usize>().unwrap_or(1).max(1);
                if count == 1 && self.current_line.is_empty() && !self.lines.is_empty() {
                    // Pop previous line so interactive progress bar rewrites work smoothly
                    self.current_line = self.lines.pop().unwrap();
                    self.cursor_col = self.current_line.chars.len();
                }
            }
            'B' => {
                // Cursor Down: \x1b[<n>B
                let count = params_str.parse::<usize>().unwrap_or(1).max(1);
                for _ in 0..count {
                    self.commit_current_line();
                }
            }
            'C' => {
                // Cursor Forward
                let count = params_str.parse::<usize>().unwrap_or(1).max(1);
                self.cursor_col += count;
            }
            'D' => {
                // Cursor Backward
                let count = params_str.parse::<usize>().unwrap_or(1).max(1);
                self.cursor_col = self.cursor_col.saturating_sub(count);
            }
            'H' | 'f' => {
                // Cursor Position: \x1b[<row>;<col>H
                if params_str.is_empty() {
                    self.cursor_col = 0;
                } else {
                    let parts: Vec<&str> = params_str.split(';').collect();
                    if parts.len() >= 2 {
                        let col = parts[1].parse::<usize>().unwrap_or(1);
                        self.cursor_col = col.saturating_sub(1);
                    } else {
                        self.cursor_col = 0;
                    }
                }
            }
            _ => {
                // Modes (?25h, ?1049h, etc.) and others are safely swallowed
            }
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
                // Standard Foreground Colors
                30 => { self.current_style = self.current_style.fg(Color::Black); idx += 1; }
                31 => { self.current_style = self.current_style.fg(Color::Red); idx += 1; }
                32 => { self.current_style = self.current_style.fg(Color::Green); idx += 1; }
                33 => { self.current_style = self.current_style.fg(Color::Yellow); idx += 1; }
                34 => { self.current_style = self.current_style.fg(Color::Blue); idx += 1; }
                35 => { self.current_style = self.current_style.fg(Color::Magenta); idx += 1; }
                36 => { self.current_style = self.current_style.fg(Color::Cyan); idx += 1; }
                37 => { self.current_style = self.current_style.fg(Color::Gray); idx += 1; }
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
                40 => { self.current_style = self.current_style.bg(Color::Black); idx += 1; }
                41 => { self.current_style = self.current_style.bg(Color::Red); idx += 1; }
                42 => { self.current_style = self.current_style.bg(Color::Green); idx += 1; }
                43 => { self.current_style = self.current_style.bg(Color::Yellow); idx += 1; }
                44 => { self.current_style = self.current_style.bg(Color::Blue); idx += 1; }
                45 => { self.current_style = self.current_style.bg(Color::Magenta); idx += 1; }
                46 => { self.current_style = self.current_style.bg(Color::Cyan); idx += 1; }
                47 => { self.current_style = self.current_style.bg(Color::Gray); idx += 1; }
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
                90 => { self.current_style = self.current_style.fg(Color::DarkGray); idx += 1; }
                91 => { self.current_style = self.current_style.fg(Color::LightRed); idx += 1; }
                92 => { self.current_style = self.current_style.fg(Color::LightGreen); idx += 1; }
                93 => { self.current_style = self.current_style.fg(Color::LightYellow); idx += 1; }
                94 => { self.current_style = self.current_style.fg(Color::LightBlue); idx += 1; }
                95 => { self.current_style = self.current_style.fg(Color::LightMagenta); idx += 1; }
                96 => { self.current_style = self.current_style.fg(Color::LightCyan); idx += 1; }
                97 => { self.current_style = self.current_style.fg(Color::White); idx += 1; }
                // Bright Background Colors
                100 => { self.current_style = self.current_style.bg(Color::DarkGray); idx += 1; }
                101 => { self.current_style = self.current_style.bg(Color::LightRed); idx += 1; }
                102 => { self.current_style = self.current_style.bg(Color::LightGreen); idx += 1; }
                103 => { self.current_style = self.current_style.bg(Color::LightYellow); idx += 1; }
                104 => { self.current_style = self.current_style.bg(Color::LightBlue); idx += 1; }
                105 => { self.current_style = self.current_style.bg(Color::LightMagenta); idx += 1; }
                106 => { self.current_style = self.current_style.bg(Color::LightCyan); idx += 1; }
                107 => { self.current_style = self.current_style.bg(Color::White); idx += 1; }
                _ => {
                    idx += 1;
                }
            }
        }
    }

    /// Total number of lines currently buffered (including the active uncommitted line).
    pub fn total_lines(&self) -> usize {
        self.lines.len() + if self.current_line.is_empty() { 0 } else { 1 }
    }

    /// User scrolled up by `delta` lines.
    pub fn scroll_up(&mut self, delta: usize) {
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
        if total > 0 {
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
            if i < self.lines.len() {
                result.push(self.lines[i].to_ratatui_line());
            } else if !self.current_line.is_empty() {
                result.push(self.current_line.to_ratatui_line());
            }
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

    /// Returns plain text representation of the last N lines.
    pub fn get_recent_plain_lines(&self, n: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let total = self.total_lines();
        let start = total.saturating_sub(n);
        for i in start..total {
            if i < self.lines.len() {
                lines.push(self.lines[i].to_plain_string());
            } else if !self.current_line.is_empty() {
                lines.push(self.current_line.to_plain_string());
            }
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
        assert_eq!(buf.lines.len(), 2);
        assert_eq!(buf.lines[0].to_plain_string(), "Hello world");
        assert_eq!(buf.lines[1].to_plain_string(), "Second line");
        assert_eq!(buf.current_line.to_plain_string(), "Third line");
    }

    #[test]
    fn test_carriage_return_overwrite() {
        let mut buf = TerminalBuffer::new(100);
        buf.push_str("Progress: 10%\rProgress: 50%\rProgress: 100%\nDone!");
        assert_eq!(buf.lines.len(), 1);
        assert_eq!(buf.lines[0].to_plain_string(), "Progress: 100%");
        assert_eq!(buf.current_line.to_plain_string(), "Done!");
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
        assert!(rat_line.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(rat_line.spans[1].content, " normal text");
    }

    #[test]
    fn test_strip_cursor_and_mode_escapes() {
        let mut buf = TerminalBuffer::new(100);
        // Feed various escape codes: hide cursor, alternate screen, OSC title, move cursor up
        buf.push_str("\x1b[?25l\x1b[?1049h\x1b]0;Antigravity\x07Clean output\x1b[?25h\n");
        assert_eq!(buf.lines.len(), 1);
        assert_eq!(buf.lines[0].to_plain_string(), "Clean output");
        assert!(!buf.lines[0].to_plain_string().contains("\x1b"));
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
}

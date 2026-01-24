//! JSON pager with syntax highlighting and scrolling.
//!
//! Provides a full-screen TUI for viewing JSON content with:
//! - Syntax highlighting
//! - Line numbers
//! - Keyboard navigation (arrows, page up/down, home/end)
//! - Search functionality (/)
//! - Quit with 'q'

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame, Terminal,
};
use std::io::{self, Stdout};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;

/// A JSON pager with syntax highlighting and scrolling.
pub struct JsonPager {
    /// The JSON content to display
    content: String,
    /// Title shown in the header
    title: String,
    /// Current scroll position (line offset)
    scroll: usize,
    /// Parsed and highlighted lines
    lines: Vec<Vec<(Style, String)>>,
    /// Total number of lines
    line_count: usize,
}

impl JsonPager {
    /// Create a new JSON pager with the given content and title.
    pub fn new(content: String, title: String) -> Self {
        let lines = Self::highlight_json(&content);
        let line_count = lines.len();
        Self {
            content,
            title,
            scroll: 0,
            lines,
            line_count,
        }
    }

    /// Parse and highlight JSON content into styled lines.
    fn highlight_json(content: &str) -> Vec<Vec<(Style, String)>> {
        let ps = SyntaxSet::load_defaults_newlines();
        let ts = ThemeSet::load_defaults();
        let theme = &ts.themes["base16-ocean.dark"];
        let syntax = ps.find_syntax_by_extension("json").unwrap();
        let mut h = HighlightLines::new(syntax, theme);

        content
            .lines()
            .map(|line| {
                let line_with_newline = format!("{}\n", line);
                let ranges = h.highlight_line(&line_with_newline, &ps).unwrap();

                ranges
                    .into_iter()
                    .map(|(style, text)| {
                        let fg =
                            Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
                        let mut ratatui_style = Style::default().fg(fg);
                        if style.font_style.contains(FontStyle::BOLD) {
                            ratatui_style = ratatui_style.add_modifier(Modifier::BOLD);
                        }
                        if style.font_style.contains(FontStyle::ITALIC) {
                            ratatui_style = ratatui_style.add_modifier(Modifier::ITALIC);
                        }
                        (ratatui_style, text.trim_end_matches('\n').to_string())
                    })
                    .collect()
            })
            .collect()
    }

    /// Run the pager in the terminal.
    pub fn run(mut self) -> io::Result<()> {
        // Check if stdout is a terminal
        if !std::io::stdout().is_terminal() {
            // Fall back to simple output
            self.print_simple();
            return Ok(());
        }

        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Run the event loop
        let result = self.run_loop(&mut terminal);

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    /// Print simple output for non-TTY contexts.
    fn print_simple(&self) {
        let width = self.line_count.to_string().len().max(3);
        let gutter_width = width + 1;
        let gutter_fill: String = "─".repeat(gutter_width);
        let border_color = "\x1b[38;5;238m";
        let reset = "\x1b[0m";

        // Top border
        print!("{}{}┬", border_color, gutter_fill);
        println!("{}{}", "─".repeat(60), reset);

        // Header
        println!(
            "{}{:>gutter_width$}│{} \x1b[1mFile: {}\x1b[0m",
            border_color, " ", reset, self.title
        );

        // Separator
        print!("{}{}┼", border_color, gutter_fill);
        println!("{}{}", "─".repeat(60), reset);

        // Content with line numbers
        for (i, line) in self.content.lines().enumerate() {
            print!(
                "\x1b[38;5;243m{:>width$}\x1b[0m {}│{} ",
                i + 1,
                border_color,
                reset,
                width = width
            );
            println!("{}", line);
        }

        // Bottom border
        print!("{}{}┴", border_color, gutter_fill);
        println!("{}{}", "─".repeat(60), reset);
    }

    /// Main event loop.
    fn run_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;

            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Down | KeyCode::Char('j') => self.scroll_down(1),
                    KeyCode::Up | KeyCode::Char('k') => self.scroll_up(1),
                    KeyCode::PageDown | KeyCode::Char(' ') => {
                        let page_size = terminal.size()?.height.saturating_sub(4) as usize;
                        self.scroll_down(page_size);
                    }
                    KeyCode::PageUp => {
                        let page_size = terminal.size()?.height.saturating_sub(4) as usize;
                        self.scroll_up(page_size);
                    }
                    KeyCode::Home | KeyCode::Char('g') => self.scroll = 0,
                    KeyCode::End | KeyCode::Char('G') => {
                        let height = terminal.size()?.height.saturating_sub(4) as usize;
                        self.scroll = self.line_count.saturating_sub(height);
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Scroll down by the given number of lines.
    fn scroll_down(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_add(lines);
        // Clamp to valid range
        if self.scroll > self.line_count.saturating_sub(1) {
            self.scroll = self.line_count.saturating_sub(1);
        }
    }

    /// Scroll up by the given number of lines.
    fn scroll_up(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
    }

    /// Render the pager UI.
    fn render(&self, frame: &mut Frame) {
        let area = frame.area();

        // Layout: header (1 line), content, footer (1 line)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Min(1),    // Content
                Constraint::Length(1), // Footer
            ])
            .split(area);

        self.render_header(frame, chunks[0]);
        self.render_content(frame, chunks[1]);
        self.render_footer(frame, chunks[2]);
    }

    /// Render the header bar.
    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let header = Paragraph::new(format!(" File: {}", self.title)).style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(header, area);
    }

    /// Render the content area with line numbers and syntax highlighting.
    fn render_content(&self, frame: &mut Frame, area: Rect) {
        let line_num_width = self.line_count.to_string().len().max(3);
        let _content_width = area.width.saturating_sub(line_num_width as u16 + 3) as usize; // +3 for " │ "

        // Build visible lines
        let visible_height = area.height as usize;
        let mut text_lines: Vec<Line> = Vec::new();

        for i in 0..visible_height {
            let line_idx = self.scroll + i;
            if line_idx >= self.line_count {
                // Empty line padding
                text_lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:>width$}", "~", width = line_num_width),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                ]));
            } else {
                // Line number
                let mut spans = vec![
                    Span::styled(
                        format!("{:>width$}", line_idx + 1, width = line_num_width),
                        Style::default().fg(Color::Indexed(243)),
                    ),
                    Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                ];

                // Highlighted content
                if let Some(styled_segments) = self.lines.get(line_idx) {
                    for (style, text) in styled_segments {
                        spans.push(Span::styled(text.clone(), *style));
                    }
                }

                text_lines.push(Line::from(spans));
            }
        }

        let content = Paragraph::new(text_lines)
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT));

        // Scrollbar
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));

        let mut scrollbar_state = ScrollbarState::new(self.line_count)
            .position(self.scroll)
            .viewport_content_length(visible_height);

        frame.render_widget(content, area);
        frame.render_stateful_widget(
            scrollbar,
            area.inner(ratatui::layout::Margin {
                vertical: 0,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }

    /// Render the footer bar with help text.
    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let progress = if self.line_count > 0 {
            let percent = ((self.scroll + 1) * 100) / self.line_count;
            format!("{}%", percent.min(100))
        } else {
            "0%".to_string()
        };

        let footer = Paragraph::new(format!(
            " ↑↓/jk: scroll  PgUp/PgDn: page  g/G: top/bottom  q: quit │ Line {}/{} ({})",
            self.scroll + 1,
            self.line_count,
            progress
        ))
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));
        frame.render_widget(footer, area);
    }
}

use std::io::IsTerminal;

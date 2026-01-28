use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;

mod sessions;
use sessions::{Session, SessionManager, SortBy};

enum Mode {
    Normal,
    Rename,
    Confirm,
    Expanded,
    Filter,
}

struct App {
    sessions: Vec<Session>,
    filtered_sessions: Vec<usize>, // Indices into sessions
    state: ListState,
    selected: Vec<usize>,
    manager: SessionManager,
    mode: Mode,
    input_buffer: String,
    confirm_message: String,
    sort_by: SortBy,
    filter_query: String,
    scroll_offset: usize,
}

impl App {
    fn new() -> io::Result<Self> {
        let manager = SessionManager::new();
        let mut sessions = manager.load_sessions()?;
        sessions.sort_by(|a, b| b.modified.cmp(&a.modified));

        let filtered_sessions: Vec<usize> = (0..sessions.len()).collect();
        let mut state = ListState::default();
        if !sessions.is_empty() {
            state.select(Some(0));
        }
        Ok(App {
            sessions,
            filtered_sessions,
            state,
            selected: Vec::new(),
            manager,
            mode: Mode::Normal,
            input_buffer: String::new(),
            confirm_message: String::new(),
            sort_by: SortBy::Date,
            filter_query: String::new(),
            scroll_offset: 0,
        })
    }

    fn reload(&mut self) -> io::Result<()> {
        self.sessions = self.manager.load_sessions()?;
        self.apply_sort();
        self.apply_filter();

        if !self.sessions.is_empty() {
            let new_idx = self.state.selected().unwrap_or(0).min(self.filtered_sessions.len().saturating_sub(1));
            self.state.select(Some(new_idx));
        } else {
            self.state.select(None);
        }
        Ok(())
    }

    fn apply_sort(&mut self) {
        match self.sort_by {
            SortBy::Date => self.sessions.sort_by(|a, b| b.modified.cmp(&a.modified)),
            SortBy::Size => self.sessions.sort_by(|a, b| b.size.cmp(&a.size)),
            SortBy::Messages => self.sessions.sort_by(|a, b| b.message_count.cmp(&a.message_count)),
        }
    }

    fn apply_filter(&mut self) {
        if self.filter_query.is_empty() {
            self.filtered_sessions = (0..self.sessions.len()).collect();
        } else {
            let query = self.filter_query.to_lowercase();
            self.filtered_sessions = self.sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.display_name().to_lowercase().contains(&query) ||
                    s.first_message.to_lowercase().contains(&query) ||
                    s.id.to_lowercase().contains(&query)
                })
                .map(|(i, _)| i)
                .collect();
        }
    }

    fn cycle_sort(&mut self) {
        self.sort_by = match self.sort_by {
            SortBy::Date => SortBy::Size,
            SortBy::Size => SortBy::Messages,
            SortBy::Messages => SortBy::Date,
        };
        self.apply_sort();
        self.apply_filter();
    }

    fn next(&mut self) {
        if self.filtered_sessions.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.filtered_sessions.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
        self.scroll_offset = 0;
    }

    fn previous(&mut self) {
        if self.filtered_sessions.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.filtered_sessions.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
        self.scroll_offset = 0;
    }

    fn toggle_select(&mut self) {
        if let Some(i) = self.state.selected() {
            if let Some(&real_idx) = self.filtered_sessions.get(i) {
                if let Some(pos) = self.selected.iter().position(|&x| x == real_idx) {
                    self.selected.remove(pos);
                } else {
                    self.selected.push(real_idx);
                }
            }
        }
    }

    fn start_rename(&mut self) {
        if let Some(i) = self.state.selected() {
            if let Some(&real_idx) = self.filtered_sessions.get(i) {
                if let Some(session) = self.sessions.get(real_idx) {
                    self.input_buffer = session.custom_name.clone().unwrap_or_default();
                    self.mode = Mode::Rename;
                }
            }
        }
    }

    fn confirm_rename(&mut self) -> io::Result<()> {
        if let Some(i) = self.state.selected() {
            if let Some(&real_idx) = self.filtered_sessions.get(i) {
                if let Some(session) = self.sessions.get(real_idx) {
                    self.manager.rename_session(&session.id, &self.input_buffer)?;
                }
            }
        }
        self.input_buffer.clear();
        self.mode = Mode::Normal;
        self.reload()?;
        Ok(())
    }

    fn start_delete(&mut self) {
        let count = if self.selected.is_empty() {
            1
        } else {
            self.selected.len()
        };
        self.confirm_message = format!("Delete {} session(s)? (y/n)", count);
        self.mode = Mode::Confirm;
    }

    fn confirm_delete(&mut self) -> io::Result<()> {
        if self.selected.is_empty() {
            if let Some(i) = self.state.selected() {
                if let Some(&real_idx) = self.filtered_sessions.get(i) {
                    self.selected.push(real_idx);
                }
            }
        }

        self.selected.sort_by(|a, b| b.cmp(a));

        for &idx in &self.selected {
            if idx < self.sessions.len() {
                self.manager.delete_session(&self.sessions[idx].id)?;
            }
        }

        self.selected.clear();
        self.mode = Mode::Normal;
        self.reload()?;
        Ok(())
    }

    fn start_filter(&mut self) {
        self.input_buffer = self.filter_query.clone();
        self.mode = Mode::Filter;
    }

    fn apply_filter_input(&mut self) {
        self.filter_query = self.input_buffer.clone();
        self.apply_filter();
        self.input_buffer.clear();
        self.mode = Mode::Normal;
        if !self.filtered_sessions.is_empty() {
            self.state.select(Some(0));
        }
    }

    fn get_preview(&self) -> String {
        if let Some(i) = self.state.selected() {
            if let Some(&real_idx) = self.filtered_sessions.get(i) {
                if let Some(session) = self.sessions.get(real_idx) {
                    let preview = format!(
                        "ID: {}\n\nSize: {}\nMessages: {}\nAge: {} days\n\nPROMPT:\n{}",
                        session.id,
                        session.size_str(),
                        session.message_count,
                        session.age_days,
                        session.first_message,
                    );

                    return preview;
                }
            }
        }
        "No session selected".to_string()
    }

    fn get_full_preview(&self) -> String {
        if let Some(i) = self.state.selected() {
            if let Some(&real_idx) = self.filtered_sessions.get(i) {
                if let Some(session) = self.sessions.get(real_idx) {
                    return self.manager.get_conversation_excerpt(&session.id, 50)
                        .unwrap_or_else(|_| "Failed to load conversation".to_string());
                }
            }
        }
        "No session selected".to_string()
    }

    fn scroll_down(&mut self) {
        self.scroll_offset += 1;
    }

    fn scroll_up(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }

    fn page_down(&mut self, page_size: usize) {
        self.scroll_offset += page_size;
    }

    fn page_up(&mut self, page_size: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(page_size);
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    match app.mode {
        Mode::Expanded => render_expanded(f, app),
        _ => {
            render_main(f, app);
            match app.mode {
                Mode::Rename => render_input_popup(f, app, "Rename Session"),
                Mode::Filter => render_input_popup(f, app, "Filter Sessions"),
                Mode::Confirm => render_confirm_popup(f, app),
                _ => {}
            }
        }
    }
}

fn render_main(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(f.area());

    // Session list
    let items: Vec<ListItem> = app
        .filtered_sessions
        .iter()
        .filter_map(|&idx| app.sessions.get(idx))
        .enumerate()
        .map(|(display_idx, s)| {
            let real_idx = app.filtered_sessions[display_idx];
            let selected = app.selected.contains(&real_idx);
            let checkbox = if selected { "[x]" } else { "[ ]" };
            let name = s.display_name();
            let line = format!("{} {} ({})", checkbox, name, s.size_str());
            ListItem::new(Line::from(Span::raw(line)))
        })
        .collect();

    let sort_indicator = match app.sort_by {
        SortBy::Date => "Date",
        SortBy::Size => "Size",
        SortBy::Messages => "Msgs",
    };

    let title = if app.filter_query.is_empty() {
        format!(" Sessions ({}) [{}] ", app.filtered_sessions.len(), sort_indicator)
    } else {
        format!(" Sessions ({}/{}) [{}] [Filter: {}] ",
            app.filtered_sessions.len(),
            app.sessions.len(),
            sort_indicator,
            app.filter_query)
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );

    f.render_stateful_widget(list, chunks[0], &mut app.state);

    // Preview pane
    let preview = Paragraph::new(app.get_preview())
        .block(Block::default().borders(Borders::ALL).title(" Preview "))
        .wrap(Wrap { trim: true });

    f.render_widget(preview, chunks[1]);

    // Help bar
    let help = Paragraph::new("↑↓:nav  Space:sel  d:del  r:rename  Enter:view  /:filter  s:sort  q:quit")
        .style(Style::default().fg(Color::DarkGray));

    let help_chunk = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area())[1];

    f.render_widget(help, help_chunk);
}

fn render_expanded(f: &mut Frame, app: &mut App) {
    let content = app.get_full_preview();
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let viewport_height = f.area().height.saturating_sub(3) as usize; // Account for borders and help

    // Clamp scroll offset to start at bottom on first render
    if app.scroll_offset == usize::MAX {
        app.scroll_offset = total_lines.saturating_sub(viewport_height);
    }
    // Clamp to valid range
    app.scroll_offset = app.scroll_offset.min(total_lines.saturating_sub(viewport_height));

    let visible_lines: Vec<Line> = lines
        .iter()
        .skip(app.scroll_offset)
        .map(|l| Line::from(Span::raw(*l)))
        .collect();

    let paragraph = Paragraph::new(visible_lines)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(format!(" Full Conversation (line {}/{}) ",
                app.scroll_offset + 1,
                lines.len().max(1))))
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, f.area());

    // Help bar
    let help = Paragraph::new("↑↓:scroll  PgUp/PgDn:page  Esc/q:back")
        .style(Style::default().fg(Color::DarkGray));

    let help_chunk = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area())[1];

    f.render_widget(help, help_chunk);
}

fn render_input_popup(f: &mut Frame, app: &App, title: &str) {
    let area = centered_rect(60, 20, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let input = Paragraph::new(app.input_buffer.as_str())
        .style(Style::default().fg(Color::Yellow));

    f.render_widget(input, inner);

    // Show cursor
    f.set_cursor_position((
        inner.x + app.input_buffer.len() as u16,
        inner.y,
    ));
}

fn render_confirm_popup(f: &mut Frame, app: &App) {
    let area = centered_rect(50, 20, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Confirm ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = Paragraph::new(app.confirm_message.as_str())
        .style(Style::default().fg(Color::Red))
        .alignment(Alignment::Center);

    f.render_widget(text, inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new()?;

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if let Event::Key(key) = event::read()? {
            match app.mode {
                Mode::Normal => match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Down | KeyCode::Char('j') => app.next(),
                    KeyCode::Up | KeyCode::Char('k') => app.previous(),
                    KeyCode::Char(' ') => app.toggle_select(),
                    KeyCode::Char('d') => app.start_delete(),
                    KeyCode::Char('r') => app.start_rename(),
                    KeyCode::Char('s') => app.cycle_sort(),
                    KeyCode::Char('/') => app.start_filter(),
                    KeyCode::Enter => {
                        app.mode = Mode::Expanded;
                        // Start at bottom - will be set properly in render_expanded
                        app.scroll_offset = usize::MAX;
                    }
                    _ => {}
                },
                Mode::Rename | Mode::Filter => match key.code {
                    KeyCode::Enter => {
                        if matches!(app.mode, Mode::Rename) {
                            app.confirm_rename()?;
                        } else {
                            app.apply_filter_input();
                        }
                    }
                    KeyCode::Esc => {
                        app.input_buffer.clear();
                        app.mode = Mode::Normal;
                    }
                    KeyCode::Char(c) => {
                        app.input_buffer.push(c);
                    }
                    KeyCode::Backspace => {
                        app.input_buffer.pop();
                    }
                    _ => {}
                },
                Mode::Confirm => match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        app.confirm_delete()?;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        app.mode = Mode::Normal;
                    }
                    _ => {}
                },
                Mode::Expanded => {
                    let page_size = (terminal.size()?.height as usize).saturating_sub(3);
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => {
                            app.mode = Mode::Normal;
                        }
                        KeyCode::Down | KeyCode::Char('j') => app.scroll_down(),
                        KeyCode::Up | KeyCode::Char('k') => app.scroll_up(),
                        KeyCode::PageDown => app.page_down(page_size),
                        KeyCode::PageUp => app.page_up(page_size),
                        _ => {}
                    }
                },
            }
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect, Alignment},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::{error::Error, io, path::PathBuf};

mod sessions;
use sessions::{Config, Session, SessionManager, SortBy};

enum Mode { Normal, Filter, Confirm, Message, PruneSelection, Expanded }
enum Action { Delete, PruneOrphans, PruneBoth }

struct App {
    sessions: Vec<Session>,
    filtered: Vec<usize>,
    state: ListState,
    selected: Vec<usize>,
    manager: SessionManager,
    mode: Mode,
    input: String,
    msg: String,
    action: Action,
    sort: SortBy,
    filter: String,
    offset: usize,
    config: Config,
    to_delete: Vec<String>,
    orphans: Vec<String>,
    cached_log: Option<Vec<String>>,
}

impl App {
    fn new() -> io::Result<Self> {
        let config = Config::load();
        let manager = SessionManager::new();
        let mut app = App {
            sessions: Vec::new(), filtered: Vec::new(), state: ListState::default(),
            selected: Vec::new(), manager, mode: Mode::Normal, input: String::new(),
            msg: String::new(), action: Action::Delete, 
            sort: config.sort_by.unwrap_or(SortBy::Date),
            filter: config.filter_query.clone().unwrap_or_default(),
            offset: 0, config, to_delete: Vec::new(), orphans: Vec::new(),
            cached_log: None,
        };
        app.reload()?;
        Ok(app)
    }

    fn reload(&mut self) -> io::Result<()> {
        self.sessions = self.manager.load_sessions()?;
        self.apply_sort();
        self.apply_filter();
        if !self.filtered.is_empty() { self.state.select(Some(0)); }
        else { self.state.select(None); }
        Ok(())
    }

    fn apply_sort(&mut self) {
        match self.sort {
            SortBy::Date => self.sessions.sort_by(|a, b| b.modified.cmp(&a.modified)),
            SortBy::Size => self.sessions.sort_by(|a, b| b.size.cmp(&a.size)),
            SortBy::Messages => self.sessions.sort_by(|a, b| b.message_count.cmp(&a.message_count)),
        }
        self.config.sort_by = Some(self.sort);
        self.config.save().ok();
    }

    fn apply_filter(&mut self) {
        let query = self.filter.to_lowercase();
        self.filtered = self.sessions.iter().enumerate()
            .filter(|(_, s)| query.is_empty() || 
                s.display_name().to_lowercase().contains(&query) || 
                s.id.to_lowercase().contains(&query) || 
                s.project.to_lowercase().contains(&query))
            .map(|(i, _)| i).collect();
        self.config.filter_query = Some(self.filter.clone());
        self.config.save().ok();
    }

    fn move_sel(&mut self, delta: isize) {
        if self.filtered.is_empty() { return; }
        let len = self.filtered.len();
        let i = match self.state.selected() {
            Some(i) => (i as isize + delta).rem_euclid(len as isize) as usize,
            None => 0,
        };
        self.state.select(Some(i));
        self.offset = 0;
    }

    fn toggle(&mut self) {
        if let Some(i) = self.state.selected() {
            let idx = self.filtered[i];
            if let Some(pos) = self.selected.iter().position(|&x| x == idx) {
                self.selected.remove(pos);
            } else {
                self.selected.push(idx);
            }
        }
    }

    fn perform_action(&mut self) -> io::Result<()> {
        match self.action {
            Action::Delete => {
                let mut report = String::from("Deleted:\n");
                for &idx in &self.selected {
                    if let Some(s) = self.sessions.get(idx) {
                        for f in self.manager.delete_session(s)? {
                            report.push_str(&format!("- {}\n", f));
                        }
                    }
                }
                self.msg = report;
                self.selected.clear();
            }
            Action::PruneOrphans => {
                let mut count = 0;
                for p in &self.orphans {
                    let path = PathBuf::from(p);
                    if path.is_dir() { std::fs::remove_dir_all(path).ok(); } 
                    else { std::fs::remove_file(path).ok(); }
                    count += 1;
                }
                self.msg = format!("Pruned {} orphans.", count);
            }
            Action::PruneBoth => {
                let mut count = 0;
                for idx in &self.selected {
                     if let Some(s) = self.sessions.get(*idx) {
                         self.manager.delete_session(s)?;
                         count += 1;
                     }
                }
                let mut orph = 0;
                for p in &self.orphans {
                    let path = PathBuf::from(p);
                    if path.is_dir() { std::fs::remove_dir_all(path).ok(); } 
                    else { std::fs::remove_file(path).ok(); }
                    orph += 1;
                }
                self.msg = format!("Deleted {} sessions, {} orphans.", count, orph);
                self.selected.clear();
            }
        }
        self.reload()?;
        self.mode = Mode::Message;
        Ok(())
    }

    fn start_export(&mut self) -> io::Result<()> {
        let mut target = Vec::new(); // Use simple vec to avoid ref issues
        if !self.selected.is_empty() {
             target = self.selected.clone();
        } else if let Some(i) = self.state.selected() {
             target.push(self.filtered[i]);
        }
        
        let dir = std::env::current_dir()?.join("exports");
        std::fs::create_dir_all(&dir)?;
        let mut count = 0;
        for idx in target {
            if let Some(s) = self.sessions.get(idx) {
                let content = self.manager.read_log(&s.path);
                std::fs::write(dir.join(format!("{}.txt", s.id)), content)?;
                count += 1;
            }
        }
        self.msg = format!("Exported {} sessions to ./exports/", count);
        self.mode = Mode::Message;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut app = App::new()?;

    let res = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    
    res
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<(), Box<dyn Error>> {
    loop {
        terminal.draw(|f| ui(f, app))?;
        if let Event::Key(key) = event::read()? {
            match app.mode {
                Mode::Normal => match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Down | KeyCode::Char('j') => app.move_sel(1),
                    KeyCode::Up | KeyCode::Char('k') => app.move_sel(-1),
                    KeyCode::Char(' ') => app.toggle(),
                    KeyCode::Char('d') => {
                        if app.selected.is_empty() { if let Some(i) = app.state.selected() { app.selected.push(app.filtered[i]); } }
                        app.to_delete.clear();
                        for &i in &app.selected { if let Some(s) = app.sessions.get(i) { app.to_delete.push(s.display_name()); } }
                        app.msg = format!("Delete {} sessions?", app.selected.len());
                        app.action = Action::Delete;
                        app.mode = Mode::Confirm;
                    },
                    KeyCode::Char('e') => { app.start_export()?; }
                    KeyCode::Char('s') => { 
                        app.sort = match app.sort { SortBy::Date=>SortBy::Size, SortBy::Size=>SortBy::Messages, _=>SortBy::Date };
                        app.apply_sort(); app.apply_filter();
                    },
                    KeyCode::Char('p') => app.mode = Mode::PruneSelection,
                    KeyCode::Char('/') => { app.input = app.filter.clone(); app.mode = Mode::Filter; }
                    KeyCode::Enter => { 
                         if let Some(i) = app.state.selected() {
                             if let Some(s) = app.sessions.get(app.filtered[i]) {
                                 let log = app.manager.read_log(&s.path);
                                 app.cached_log = Some(log.lines().map(String::from).collect());
                                 app.offset = usize::MAX; // Will be clamped in render
                                 app.mode = Mode::Expanded;
                             }
                         }
                    },
                    _ => {}
                },
                Mode::Filter => match key.code {
                    KeyCode::Enter => { app.filter = app.input.clone(); app.apply_filter(); app.mode = Mode::Normal; }
                    KeyCode::Esc => { app.mode = Mode::Normal; }
                    KeyCode::Char(c) => app.input.push(c),
                    KeyCode::Backspace => { app.input.pop(); },
                    _ => {}
                },
                Mode::Confirm => match key.code {
                    KeyCode::Char('y')|KeyCode::Char('Y') => app.perform_action()?,
                    KeyCode::Esc|KeyCode::Char('n') => app.mode = Mode::Normal,
                    _ => {}
                },
                Mode::Message => if matches!(key.code, KeyCode::Enter|KeyCode::Esc) { app.mode = Mode::Normal; },
                Mode::Expanded => match key.code {
                    KeyCode::Esc|KeyCode::Char('q') => {
                        app.cached_log = None;
                        app.mode = Mode::Normal;
                    },
                    KeyCode::Down|KeyCode::Char('j') => app.offset += 1,
                    KeyCode::Up|KeyCode::Char('k') => app.offset = app.offset.saturating_sub(1),
                    KeyCode::PageUp => app.offset = app.offset.saturating_sub(20),
                    KeyCode::PageDown => app.offset += 20,
                    _ => {}
                },
                Mode::PruneSelection => match key.code {
                    KeyCode::Esc => app.mode = Mode::Normal,
                    KeyCode::Char('1') => { // Empty
                        app.selected = app.sessions.iter().enumerate().filter(|(_,s)| s.message_count==0).map(|(i,_)| i).collect();
                        if app.selected.is_empty() { app.msg="No empty sessions.".into(); app.mode=Mode::Message; }
                        else { app.msg=format!("Delete {} empty sessions?", app.selected.len()); app.action=Action::Delete; app.mode=Mode::Confirm; }
                    },
                    KeyCode::Char('2') => { // Orphans
                        app.orphans = app.manager.find_orphans().iter().map(|p| p.to_string_lossy().into()).collect();
                        if app.orphans.is_empty() { app.msg="No orphans.".into(); app.mode=Mode::Message; }
                        else { app.to_delete=app.orphans.clone(); app.msg=format!("Delete {} orphans?", app.orphans.len()); app.action=Action::PruneOrphans; app.mode=Mode::Confirm; }
                    },
                    KeyCode::Char('3') => { // Both
                        app.selected = app.sessions.iter().enumerate().filter(|(_,s)| s.message_count==0).map(|(i,_)| i).collect();
                        app.orphans = app.manager.find_orphans().iter().map(|p| p.to_string_lossy().into()).collect();
                        if app.selected.is_empty() && app.orphans.is_empty() { app.msg="Nothing to prune.".into(); app.mode=Mode::Message; }
                        else { app.msg=format!("Delete {} empty & {} orphans?", app.selected.len(), app.orphans.len()); app.action=Action::PruneBoth; app.mode=Mode::Confirm; }
                    },
                    KeyCode::Char('4') => { // History
                         let c = app.manager.prune_history_orphans();
                         app.msg = format!("Pruned {} history entries.", c);
                         app.mode = Mode::Message;
                    },
                    _ => {}
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(main_layout[0]);

    let items: Vec<ListItem> = app.filtered.iter().map(|&i| {
        let s = &app.sessions[i];
        let mark = if app.selected.contains(&i) { "[x]" } else { "[ ]" };
        let msgs = if s.message_count > 0 { format!("{} msgs", s.message_count) } else { "empty".to_string() };
        ListItem::new(format!("{} {} ({}, {})", mark, s.display_name(), s.size_str(), msgs))
    }).collect();

    let title = format!(" Sessions ({}/{}) Filter:[{}] Sort:[{:?}] ", 
        app.filtered.len(), app.sessions.len(), app.filter, app.sort);
    
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title).title_alignment(Alignment::Center))
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
    f.render_stateful_widget(list, chunks[0], &mut app.state);

    let preview_text = if let Some(i) = app.state.selected() {
        if let Some(s) = app.sessions.get(app.filtered[i]) {
            let todos = s.get_todos();
            let mut info = format!("ID: {}\nProject: {}\nSize: {}\nModified: {}\n", 
                s.id, s.project, s.size_str(), s.formatted_age());
            
            if s.message_count > 0 {
                info.push_str(&format!("Messages: {}\n", s.message_count));
            }
            if !todos.is_empty() {
                info.push_str(&format!("\nTODO:\n- {}\n", todos.join("\n- ")));
            }
            if !s.first_message.is_empty() {
                info.push_str(&format!("\nPROMPT:\n{}", s.first_message));
            }
            info
        } else { String::new() }
    } else { String::new() };

    f.render_widget(Paragraph::new(preview_text).block(Block::default().borders(Borders::ALL).title(" Preview ")).wrap(Wrap{trim:true}), chunks[1]);
    
    // Help bar
    let help_text = "q:Quit j/k:Nav Space:Sel d:Del e:Exp s:Sort p:Prune /:Filt Enter:Open";
    f.render_widget(Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray).bg(Color::Black)), main_layout[1]);

    // Popup logic
    let area = f.area();
    match app.mode {
        Mode::Filter => {
             let r = centered(60, 10, area);
             f.render_widget(Clear, r);
             let b = Block::default().borders(Borders::ALL).title(" Filter Sessions ");
             let inner_area = b.inner(r);
             f.render_widget(b, r);
             f.render_widget(Paragraph::new(app.input.as_str()).style(Style::default().fg(Color::Yellow)), inner_area);
        },
        Mode::Confirm => {
             let r = centered(60, 60, area);
             f.render_widget(Clear, r);
             let b = Block::default().borders(Borders::ALL).title(" Confirm Action ").style(Style::default().bg(Color::Black));
             let inner_area = b.inner(r);
             f.render_widget(b, r);
             
             let l = Layout::default()
                 .constraints([Constraint::Length(2), Constraint::Min(0), Constraint::Length(2)])
                 .split(inner_area);
             
             f.render_widget(Paragraph::new(app.msg.as_str()).style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)).alignment(Alignment::Center), l[0]);
             
             let del_items: Vec<ListItem> = app.to_delete.iter()
                 .map(|s| ListItem::new(Line::from(vec![
                     ratatui::text::Span::styled("- ", Style::default().fg(Color::DarkGray)),
                     ratatui::text::Span::raw(s)
                 ])))
                 .collect();
             
             f.render_widget(List::new(del_items).block(Block::default().borders(Borders::TOP).title(" Items to delete ")), l[1]);
             
             f.render_widget(Paragraph::new("Press Y to Confirm, N to Cancel").alignment(Alignment::Center).style(Style::default().fg(Color::DarkGray)), l[2]);
        },
        Mode::Message => {
             let r = centered(50, 20, area);
             f.render_widget(Clear, r);
             let b = Block::default().borders(Borders::ALL).title(" Information ");
             f.render_widget(b.clone(), r);
             f.render_widget(Paragraph::new(app.msg.as_str()).wrap(Wrap{trim:true}).block(Block::default().padding(ratatui::widgets::Padding::new(2,2,1,1))), r);
        },
        Mode::PruneSelection => {
             let r = centered(40, 30, area);
             f.render_widget(Clear, r);
             let b = Block::default().title(" Prune Options ").borders(Borders::ALL);
             let inner_area = b.inner(r);
             f.render_widget(b, r);
             let text = vec![
                 Line::from(" [1] Empty Sessions"),
                 Line::from(" [2] Orphaned Files"),
                 Line::from(" [3] Both"),
                 Line::from(" [4] Prune History"),
                 Line::from(""),
                 Line::from(ratatui::text::Span::styled(" Esc to Cancel", Style::default().fg(Color::DarkGray))),
             ];
             f.render_widget(Paragraph::new(text).block(Block::default().padding(ratatui::widgets::Padding::new(2,2,2,1))), inner_area);
        },
        Mode::Expanded => {
             if let Some(lines) = &app.cached_log {
                 let h = area.height as usize - 2;
                 if app.offset == usize::MAX { app.offset = lines.len().saturating_sub(h); }
                 app.offset = app.offset.min(lines.len().saturating_sub(h));
                 
                 let v: Vec<Line> = lines.iter()
                     .skip(app.offset)
                     .take(h)
                     .map(|l| Line::from(l.as_str()))
                     .collect();
                 
                 f.render_widget(Clear, area);
                 let b = Block::default().borders(Borders::ALL)
                     .title(format!(" Full Log (Line {}/{}) ", app.offset, lines.len()));
                 f.render_widget(Paragraph::new(v).block(b).wrap(Wrap{trim:false}), area);
             }
        },
        _ => {}
    }
}

fn centered(px: u16, py: u16, r: Rect) -> Rect {
    let v = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage((100-py)/2), Constraint::Percentage(py), Constraint::Percentage((100-py)/2)]).split(r);
    Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage((100-px)/2), Constraint::Percentage(px), Constraint::Percentage((100-px)/2)]).split(v[1])[1]
}

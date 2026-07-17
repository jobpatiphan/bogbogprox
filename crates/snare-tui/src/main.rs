//! `snare-tui` — terminal UI over the daemon's REST API (§5.1).

mod httpclient;

use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use snare_core::model::{Flow, FlowSummary};

#[derive(Parser)]
#[command(name = "snare-tui", version, about = "Snare terminal UI")]
struct Cli {
    /// API host.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// API port.
    #[arg(long, default_value_t = 9000)]
    port: u16,
}

struct App {
    host: String,
    port: u16,
    flows: Vec<FlowSummary>,
    state: ListState,
    detail: Option<Flow>,
    status: String,
    last_refresh: Instant,
}

impl App {
    fn new(host: String, port: u16) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            host,
            port,
            flows: Vec::new(),
            state,
            detail: None,
            status: "loading…".into(),
            last_refresh: Instant::now() - Duration::from_secs(10),
        }
    }

    fn refresh(&mut self) {
        match httpclient::get_json::<Vec<FlowSummary>>(
            &self.host,
            self.port,
            "/api/v1/flows?limit=500",
        ) {
            Ok(flows) => {
                self.status = format!("{} flows", flows.len());
                self.flows = flows;
                if self.state.selected().unwrap_or(0) >= self.flows.len() {
                    self.state.select(if self.flows.is_empty() {
                        None
                    } else {
                        Some(self.flows.len() - 1)
                    });
                }
                self.load_detail();
            }
            Err(e) => self.status = format!("error: {e}"),
        }
        self.last_refresh = Instant::now();
    }

    fn load_detail(&mut self) {
        let Some(i) = self.state.selected() else {
            self.detail = None;
            return;
        };
        let Some(f) = self.flows.get(i) else {
            self.detail = None;
            return;
        };
        let path = format!("/api/v1/flows/{}", f.id);
        self.detail = httpclient::get_json::<Flow>(&self.host, self.port, &path).ok();
    }

    fn move_by(&mut self, delta: isize) {
        if self.flows.is_empty() {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.flows.len() as isize - 1) as usize;
        self.state.select(Some(next));
        self.load_detail();
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(cli.host, cli.port);
    let res = run(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

fn run<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    loop {
        if app.last_refresh.elapsed() > Duration::from_secs(2) {
            app.refresh();
        }
        terminal.draw(|f| draw(f, app))?;

        if event::poll(Duration::from_millis(400))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('j') | KeyCode::Down => app.move_by(1),
                    KeyCode::Char('k') | KeyCode::Up => app.move_by(-1),
                    KeyCode::Char('g') => {
                        app.state.select(Some(0));
                        app.load_detail();
                    }
                    KeyCode::Char('G') => {
                        if !app.flows.is_empty() {
                            app.state.select(Some(app.flows.len() - 1));
                            app.load_detail();
                        }
                    }
                    KeyCode::Char('r') => app.refresh(),
                    _ => {}
                }
            }
        }
    }
}

fn draw(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let title = Line::from(vec![
        Span::styled(" 🪤 Snare ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(&app.status, Style::default().fg(Color::Cyan)),
    ]);
    f.render_widget(Paragraph::new(title), chunks[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(chunks[1]);

    let items: Vec<ListItem> = app
        .flows
        .iter()
        .map(|fl| {
            let status = fl.status.map(|s| s.to_string()).unwrap_or_else(|| "···".into());
            let color = match fl.status {
                Some(s) if s < 300 => Color::Green,
                Some(s) if s < 400 => Color::Cyan,
                Some(s) if s < 500 => Color::Yellow,
                Some(_) => Color::Red,
                None => Color::DarkGray,
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{status:>3} "), Style::default().fg(color)),
                Span::styled(format!("{:<5} ", fl.method), Style::default().fg(Color::Magenta)),
                Span::raw(format!("{}{}", fl.host, fl.path)),
            ]))
        })
        .collect();

    let mut list_state = app.state.clone();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" flows "))
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, body[0], &mut list_state);

    f.render_widget(detail_widget(app), body[1]);

    let help = Line::from(Span::styled(
        " j/k move · g/G top/bottom · r refresh · q quit ",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(help), chunks[2]);
}

fn detail_widget<'a>(app: &'a App) -> Paragraph<'a> {
    let mut lines: Vec<Line> = Vec::new();
    if let Some(flow) = &app.detail {
        let req = &flow.request;
        lines.push(Line::from(Span::styled(
            format!("{} {}", req.method, req.url()),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));
        for (k, v) in req.headers.iter().take(20) {
            lines.push(Line::from(vec![
                Span::styled(format!("{k}: "), Style::default().fg(Color::Blue)),
                Span::raw(v.clone()),
            ]));
        }
        if !req.body.is_empty() {
            lines.push(Line::from(""));
            lines.push(body_preview("request body", &req.body));
        }
        lines.push(Line::from(""));
        if let Some(resp) = &flow.response {
            lines.push(Line::from(Span::styled(
                format!("← {} ({} bytes)", resp.status, resp.body.len()),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            )));
            for (k, v) in resp.headers.iter().take(20) {
                lines.push(Line::from(vec![
                    Span::styled(format!("{k}: "), Style::default().fg(Color::Blue)),
                    Span::raw(v.clone()),
                ]));
            }
            if !resp.body.is_empty() {
                lines.push(Line::from(""));
                lines.push(body_preview("response body", &resp.body));
            }
        } else {
            lines.push(Line::from(Span::styled(
                "(awaiting response…)",
                Style::default().fg(Color::DarkGray),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "select a flow — or start browsing through the proxy",
            Style::default().fg(Color::DarkGray),
        )));
    }
    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" detail "))
        .wrap(Wrap { trim: false })
}

fn body_preview<'a>(label: &str, body: &[u8]) -> Line<'a> {
    let text = String::from_utf8_lossy(body);
    let snippet: String = text.chars().take(2000).collect();
    Line::from(vec![
        Span::styled(format!("[{label}] "), Style::default().fg(Color::DarkGray)),
        Span::raw(snippet),
    ])
}

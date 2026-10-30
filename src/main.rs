use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;
use trilogy_save_editor::save::{Field, Game, InputKind, Row};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("-h" | "--help") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(path) if args.next().is_none() => match run(Some(PathBuf::from(path))) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err:#}");
                ExitCode::FAILURE
            }
        },
        Some(_) => {
            eprintln!("error: expected one save path");
            print_help();
            ExitCode::FAILURE
        }
        None => match run(None) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err:#}");
                ExitCode::FAILURE
            }
        },
    }
}

fn print_help() {
    println!(
        "\
trilogy-save-editor [SAVE]

Edit a Mass Effect Legendary Edition PC save from the terminal.
Shepard's level, XP, credits, resources, morality, and difficulty.

With no path, the program asks for one.

Keys
  Up/Down     move
  Left/Right  change a choice (gender, origin, class, difficulty)
  Enter       type a name or a number
  s           save (the original is copied to SAVE.bak the first time)
  q           quit"
    );
}

fn run(path: Option<PathBuf>) -> Result<()> {
    if !io::stdout().is_terminal() {
        anyhow::bail!("This editor runs in a terminal.");
    }
    let screen = match path {
        Some(path) => {
            let game = Game::load_path(&path)?;
            Screen::Editor(Editor::new(path, game))
        }
        None => Screen::Prompt { buffer: String::new(), error: None },
    };
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, screen);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, mut screen: Screen) -> Result<()> {
    loop {
        terminal.draw(|frame| render(frame, &mut screen))?;
        let event = event::read().context("Could not read keyboard input")?;
        let Event::Key(key) = event else { continue };
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            continue;
        }
        if screen.handle(key) {
            break;
        }
    }
    Ok(())
}

enum Screen {
    Prompt { buffer: String, error: Option<String> },
    Editor(Editor),
}

impl Screen {
    fn handle(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }
        match self {
            Screen::Prompt { .. } => handle_prompt(self, key),
            Screen::Editor(editor) => editor.handle(key),
        }
    }
}

fn handle_prompt(screen: &mut Screen, key: KeyEvent) -> bool {
    if key.code == KeyCode::Enter {
        let text = match screen {
            Screen::Prompt { buffer, .. } => buffer.trim().to_string(),
            Screen::Editor(_) => return false,
        };
        if text.is_empty() {
            if let Screen::Prompt { error, .. } = screen {
                *error = Some("Enter a path to a .pcsav file.".to_string());
            }
            return false;
        }
        let path = expand_user_path(&text);
        match Game::load_path(&path) {
            Ok(game) => *screen = Screen::Editor(Editor::new(path, game)),
            Err(err) => {
                if let Screen::Prompt { error, .. } = screen {
                    *error = Some(format!("{err:#}"));
                }
            }
        }
        return false;
    }

    let Screen::Prompt { buffer, error } = screen else { return false };
    match key.code {
        KeyCode::Esc => return true,
        KeyCode::Backspace => {
            buffer.pop();
            *error = None;
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            buffer.push(ch);
            *error = None;
        }
        _ => {}
    }
    false
}

struct Editor {
    path: PathBuf,
    game: Game,
    cursor: usize,
    scroll: usize,
    editing: Option<String>,
    confirm_quit: bool,
    dirty: bool,
    status: Status,
}

enum Status {
    Hint,
    Error(String),
    Saved(String),
}

impl Editor {
    fn new(path: PathBuf, game: Game) -> Self {
        Self {
            path,
            game,
            cursor: 0,
            scroll: 0,
            editing: None,
            confirm_quit: false,
            dirty: false,
            status: Status::Hint,
        }
    }

    fn handle(&mut self, key: KeyEvent) -> bool {
        if self.confirm_quit {
            return self.handle_confirm(key);
        }
        if self.editing.is_some() {
            self.handle_edit(key);
            return false;
        }
        match key.code {
            KeyCode::Esc => false,
            KeyCode::Char('q') => {
                if self.dirty {
                    self.confirm_quit = true;
                    false
                } else {
                    true
                }
            }
            KeyCode::Char('s') => {
                self.save_file();
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_cursor(-1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_cursor(1);
                false
            }
            KeyCode::PageUp => {
                self.move_cursor(-8);
                false
            }
            KeyCode::PageDown => {
                self.move_cursor(8);
                false
            }
            KeyCode::Home => {
                self.cursor = 0;
                false
            }
            KeyCode::End => {
                self.cursor = self.game.rows().len().saturating_sub(1);
                false
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.cycle(-1);
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.cycle(1);
                false
            }
            KeyCode::Enter => {
                self.begin_edit();
                false
            }
            _ => false,
        }
    }

    fn handle_confirm(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => true,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.confirm_quit = false;
                false
            }
            _ => false,
        }
    }

    fn handle_edit(&mut self, key: KeyEvent) {
        let Some(buffer) = self.editing.as_mut() else { return };
        match key.code {
            KeyCode::Esc => self.editing = None,
            KeyCode::Enter => {
                let text = buffer.clone();
                let field = self.current_field();
                self.editing = None;
                match self.game.apply_text(field, &text) {
                    Ok(()) => {
                        self.dirty = true;
                        self.status = Status::Hint;
                    }
                    Err(err) => self.status = Status::Error(err.to_string()),
                }
            }
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => buffer.push(ch),
            _ => {}
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.game.rows().len();
        if len == 0 {
            return;
        }
        let next = self.cursor as isize + delta;
        self.cursor = next.clamp(0, len as isize - 1) as usize;
    }

    fn current_field(&self) -> Field {
        self.game.rows()[self.cursor].field
    }

    fn cycle(&mut self, delta: i32) {
        let field = self.current_field();
        match self.game.cycle(field, delta) {
            Ok(()) => {
                self.dirty = true;
                self.status = Status::Hint;
            }
            Err(err) => self.status = Status::Error(err.to_string()),
        }
    }

    fn begin_edit(&mut self) {
        let field = self.current_field();
        match self.game.input_kind(field) {
            InputKind::Choice => self.cycle(1),
            InputKind::Readonly => self.cycle(1),
            InputKind::Text | InputKind::Int | InputKind::Float => match self.game.value(field) {
                Some(value) => {
                    self.editing = Some(value);
                    self.status = Status::Hint;
                }
                None => {
                    self.status =
                        Status::Error(format!("This save has no {} value.", field.label()));
                }
            },
        }
    }

    fn save_file(&mut self) {
        match self.game.save_to(&self.path) {
            Ok(report) => {
                self.dirty = false;
                self.status = Status::Saved(match report.backup {
                    Some(path) => format!("Saved. Original copied to {}", path.display()),
                    None => "Saved".to_string(),
                });
            }
            Err(err) => self.status = Status::Error(format!("{err:#}")),
        }
    }
}

fn render(frame: &mut Frame, screen: &mut Screen) {
    match screen {
        Screen::Prompt { buffer, error } => render_prompt(frame, buffer, error.as_deref()),
        Screen::Editor(editor) => render_editor(frame, editor),
    }
}

fn render_prompt(frame: &mut Frame, buffer: &str, error: Option<&str>) {
    let [title, input, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(frame.area());

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Mass Effect save editor",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from("PC Legendary Edition"),
        ]),
        title,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Path to a .pcsav file"),
            Line::from(Span::styled(format!("> {buffer}_"), Style::default().fg(Color::Cyan))),
        ]),
        input,
    );
    let mut lines = vec![Line::from(Span::styled(
        "Example: ~/Documents/BioWare/Mass Effect Legendary Edition/Save/...",
        Style::default().fg(Color::DarkGray),
    ))];
    if let Some(error) = error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(error.to_string(), Style::default().fg(Color::Red))));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
    frame.render_widget(Paragraph::new("enter open    esc quit"), footer);
}

fn render_editor(frame: &mut Frame, editor: &mut Editor) {
    let footer_height = if editor.editing.is_some() || editor.confirm_quit { 3 } else { 2 };
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(footer_height),
    ])
    .areas(frame.area());

    let name = editor.game.value(Field::Name).unwrap_or_else(|| "Shepard".to_string());
    let level = editor.game.value(Field::Level).unwrap_or_else(|| "?".to_string());
    let dirty = if editor.dirty { "  *" } else { "" };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    editor.game.title(),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("    {name}    Level {level}")),
                Span::styled(dirty, Style::default().fg(Color::Yellow)),
            ]),
            Line::from(Span::styled(
                editor.path.display().to_string(),
                Style::default().fg(Color::DarkGray),
            )),
        ]),
        header,
    );

    let rows = editor.game.rows();
    let items = editor_lines(&editor.game, &rows, editor.cursor);
    let cursor_line = items.iter().position(|(row, _)| *row == Some(editor.cursor)).unwrap_or(0);
    let height = body.height as usize;
    ensure_visible(&mut editor.scroll, cursor_line, height);
    let visible: Vec<Line> =
        items.into_iter().skip(editor.scroll).take(height).map(|(_, line)| line).collect();
    frame.render_widget(Paragraph::new(visible), body);

    frame.render_widget(Paragraph::new(footer_lines(editor)).wrap(Wrap { trim: false }), footer);
}

fn editor_lines(game: &Game, rows: &[Row], cursor: usize) -> Vec<(Option<usize>, Line<'static>)> {
    let mut items = Vec::new();
    let mut last_group = "";
    for (index, row) in rows.iter().enumerate() {
        if row.group != last_group {
            items.push((
                None,
                Line::from(Span::styled(
                    format!(" {group}", group = row.group),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )),
            ));
            last_group = row.group;
        }
        let value = game.value(row.field).unwrap_or_else(|| "n/a".to_string());
        let selected = index == cursor;
        let text = format!(
            "{marker} {label:<20} {value}",
            marker = if selected { ">" } else { " " },
            label = row.field.label()
        );
        let style = if selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        items.push((Some(index), Line::from(Span::styled(text, style))));
    }
    items
}

fn footer_lines(editor: &Editor) -> Vec<Line<'static>> {
    if editor.confirm_quit {
        return vec![
            Line::from("y quit    n stay"),
            Line::from(Span::styled("Quit without saving?", Style::default().fg(Color::Yellow))),
        ];
    }
    if let Some(buffer) = &editor.editing {
        let field = editor.current_field();
        return vec![
            Line::from("enter confirm    esc cancel"),
            Line::from(Span::styled(
                format!("{}: {buffer}_", field.label()),
                Style::default().fg(Color::Cyan),
            )),
        ];
    }
    let keys = Line::from("↑↓ move    ←→ change    enter edit    s save    q quit");
    let status = match &editor.status {
        Status::Hint => Line::from(Span::styled(
            "Changes stay in memory until you save.",
            Style::default().fg(Color::DarkGray),
        )),
        Status::Error(text) => {
            Line::from(Span::styled(text.clone(), Style::default().fg(Color::Red)))
        }
        Status::Saved(text) => {
            Line::from(Span::styled(text.clone(), Style::default().fg(Color::Green)))
        }
    };
    vec![keys, status]
}

fn ensure_visible(scroll: &mut usize, cursor_line: usize, height: usize) {
    if height == 0 {
        return;
    }
    if cursor_line < *scroll {
        *scroll = cursor_line;
    } else if cursor_line >= *scroll + height {
        *scroll = cursor_line + 1 - height;
    }
}

fn expand_user_path(text: &str) -> PathBuf {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if trimmed == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(trimmed)
}

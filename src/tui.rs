use crate::{
    agent::{Agent, ApprovalDecision, RuntimeYield},
    domain::{Message, Role, Session, ToolInvocation},
    provider::ModelProvider,
    store::SessionStore,
    terminal_text::sanitize_terminal,
};
use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::{
    io::{self, IsTerminal, Write},
    path::Path,
    time::Duration,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub async fn run<P>(agent: &Agent<P>, workspace: &Path, model: &str) -> Result<()>
where
    P: ModelProvider,
{
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!("Ikaros requires an interactive terminal");
    }
    let session = resolve_session(agent.store(), workspace, model)?;
    let mut app = UiApp::new(session, agent.store())?;
    let initial = agent.resume(&app.session.id).await?;
    app.apply_yield(initial, agent.store())?;

    let _terminal_session = TerminalSession::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
    terminal.clear()?;

    loop {
        terminal.draw(|frame| render(frame, &app))?;
        if app.should_quit {
            break;
        }
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if app.modal.is_some() {
            handle_modal_key(&mut terminal, &mut app, agent, key).await?;
        } else {
            handle_normal_key(&mut terminal, &mut app, agent, key).await?;
        }
    }
    terminal.show_cursor()?;
    Ok(())
}

async fn handle_normal_key<B, P>(
    terminal: &mut Terminal<B>,
    app: &mut UiApp,
    agent: &Agent<P>,
    key: KeyEvent,
) -> Result<()>
where
    B: Backend,
    P: ModelProvider,
{
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => app.should_quit = true,
        KeyCode::Backspace => {
            app.input.pop();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => app.input.clear(),
        KeyCode::Char(character) => app.input.push(character),
        KeyCode::Enter => {
            let input = std::mem::take(&mut app.input);
            let input = input.trim();
            if input.is_empty() {
                return Ok(());
            }
            app.transcript.push(Message::user(input));
            app.status = "thinking".to_owned();
            terminal.draw(|frame| render(frame, app))?;
            let outcome = agent.start_turn(&app.session.id, input).await;
            match outcome {
                Ok(outcome) => app.apply_yield(outcome, agent.store())?,
                Err(error) => {
                    app.reload(agent.store())?;
                    app.notice = Some(format!("error: {error:#}"));
                    app.status = "error".to_owned();
                }
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_modal_key<B, P>(
    terminal: &mut Terminal<B>,
    app: &mut UiApp,
    agent: &Agent<P>,
    key: KeyEvent,
) -> Result<()>
where
    B: Backend,
    P: ModelProvider,
{
    match key.code {
        KeyCode::Up => {
            app.modal_scroll = app.modal_scroll.saturating_sub(1);
            return Ok(());
        }
        KeyCode::Down => {
            app.modal_scroll = app.modal_scroll.saturating_add(1);
            return Ok(());
        }
        KeyCode::PageUp => {
            app.modal_scroll = app.modal_scroll.saturating_sub(10);
            return Ok(());
        }
        KeyCode::PageDown => {
            app.modal_scroll = app.modal_scroll.saturating_add(10);
            return Ok(());
        }
        _ => {}
    }
    let Some(modal) = app.modal.clone() else {
        return Ok(());
    };
    match modal {
        Modal::Approval(invocation) => {
            let decision = match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => Some(ApprovalDecision::Approve),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    Some(ApprovalDecision::Deny)
                }
                _ => None,
            };
            if let Some(decision) = decision {
                app.modal = None;
                app.status = match decision {
                    ApprovalDecision::Approve => "running approved tool".to_owned(),
                    ApprovalDecision::Deny => "denying tool".to_owned(),
                };
                terminal.draw(|frame| render(frame, app))?;
                let outcome = agent
                    .decide(&app.session.id, &invocation.id, decision)
                    .await;
                match outcome {
                    Ok(outcome) => app.apply_yield(outcome, agent.store())?,
                    Err(error) => {
                        app.reload(agent.store())?;
                        app.notice = Some(format!("error: {error:#}"));
                        app.status = "error".to_owned();
                    }
                }
            }
        }
        Modal::Recovery(invocation) => match key.code {
            KeyCode::Char('f') | KeyCode::Char('F') => {
                app.modal = None;
                app.status = "marking interrupted tool failed".to_owned();
                terminal.draw(|frame| render(frame, app))?;
                let outcome = agent
                    .recover_interrupted(&app.session.id, &invocation.id)
                    .await;
                match outcome {
                    Ok(outcome) => app.apply_yield(outcome, agent.store())?,
                    Err(error) => {
                        app.reload(agent.store())?;
                        app.notice = Some(format!("error: {error:#}"));
                        app.status = "error".to_owned();
                    }
                }
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                app.should_quit = true;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.should_quit = true;
            }
            _ => {}
        },
    }
    Ok(())
}

fn resolve_session(store: &SessionStore, workspace: &Path, model: &str) -> Result<Session> {
    store
        .latest_session(workspace)?
        .map_or_else(|| store.create_session(workspace, model), Ok)
}

#[derive(Debug, Clone)]
enum Modal {
    Approval(ToolInvocation),
    Recovery(ToolInvocation),
}

struct UiApp {
    session: Session,
    transcript: Vec<Message>,
    input: String,
    status: String,
    notice: Option<String>,
    modal: Option<Modal>,
    modal_scroll: u16,
    should_quit: bool,
}

impl UiApp {
    fn new(session: Session, store: &SessionStore) -> Result<Self> {
        let transcript = store.messages(&session.id)?;
        Ok(Self {
            session,
            transcript,
            input: String::new(),
            status: "ready".to_owned(),
            notice: None,
            modal: None,
            modal_scroll: 0,
            should_quit: false,
        })
    }

    fn reload(&mut self, store: &SessionStore) -> Result<()> {
        self.transcript = store.messages(&self.session.id)?;
        Ok(())
    }

    fn apply_yield(&mut self, outcome: RuntimeYield, store: &SessionStore) -> Result<()> {
        self.reload(store)?;
        match outcome {
            RuntimeYield::Ready => {
                self.status = "ready".to_owned();
                self.modal = None;
            }
            RuntimeYield::Completed(_) => {
                self.status = "ready".to_owned();
                self.modal = None;
            }
            RuntimeYield::AwaitingApproval(invocation) => {
                self.status = "approval required".to_owned();
                self.modal = Some(Modal::Approval(invocation));
                self.modal_scroll = 0;
            }
            RuntimeYield::RecoveryRequired(invocation) => {
                self.status = "interrupted tool requires recovery".to_owned();
                self.modal = Some(Modal::Recovery(invocation));
                self.modal_scroll = 0;
            }
        }
        Ok(())
    }
}

fn render(frame: &mut Frame<'_>, app: &UiApp) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(frame.area());
    let content_width = layout[0].width.saturating_sub(2);
    let lines = wrap_transcript_lines(transcript_lines(app), content_width);
    let visible_height = layout[0].height.saturating_sub(2) as usize;
    let scroll = lines
        .len()
        .saturating_sub(visible_height)
        .min(u16::MAX as usize) as u16;
    let transcript = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Ikaros · {} ", short_id(&app.session.id))),
        )
        .scroll((scroll, 0));
    frame.render_widget(transcript, layout[0]);

    let input = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Message "));
    frame.render_widget(input, layout[1]);
    if app.modal.is_none() {
        let requested_cursor_x = layout[1].x.saturating_add(1).saturating_add(
            UnicodeWidthStr::width(app.input.as_str()).min(u16::MAX as usize) as u16,
        );
        let cursor_x = requested_cursor_x.min(layout[1].right().saturating_sub(2));
        frame.set_cursor_position((cursor_x, layout[1].y.saturating_add(1)));
    }

    let status = Line::from(vec![
        Span::styled(
            " status ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::raw(format!(" {}", sanitize_terminal(&app.status))),
    ]);
    frame.render_widget(Paragraph::new(status), layout[2]);

    if let Some(modal) = &app.modal {
        render_modal(frame, modal, app.modal_scroll);
    }
}

fn transcript_lines(app: &UiApp) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(notice) = &app.notice {
        push_prefixed_lines(
            &mut lines,
            "Info",
            notice,
            Style::default().fg(Color::Yellow),
        );
        lines.push(Line::default());
    }
    for message in &app.transcript {
        match message.role {
            Role::User => push_prefixed_lines(
                &mut lines,
                "You",
                message.content.as_deref().unwrap_or_default(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Role::Assistant => {
                if let Some(content) = &message.content {
                    push_prefixed_lines(
                        &mut lines,
                        "Ikaros",
                        content,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    );
                }
                for call in &message.tool_calls {
                    push_prefixed_lines(
                        &mut lines,
                        "Tool request",
                        &format!(
                            "{} {}",
                            call.name,
                            serde_json::to_string(&call.arguments)
                                .unwrap_or_else(|_| "<invalid arguments>".to_owned())
                        ),
                        Style::default().fg(Color::Magenta),
                    );
                }
            }
            Role::Tool => push_prefixed_lines(
                &mut lines,
                "Tool",
                message.content.as_deref().unwrap_or_default(),
                Style::default().fg(Color::Blue),
            ),
            Role::System => {}
        }
        lines.push(Line::default());
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "Type a message. Every tool call requires explicit approval.",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn wrap_transcript_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let mut wrapped = Vec::new();
    for line in lines {
        if line.spans.is_empty() {
            wrapped.push(Line::default());
            continue;
        }
        let mut output_spans = Vec::new();
        let mut output_width: usize = 0;
        for span in line.spans {
            let mut text = String::new();
            for character in span.content.chars() {
                let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
                if output_width > 0 && output_width.saturating_add(character_width) > width {
                    if !text.is_empty() {
                        output_spans.push(Span::styled(std::mem::take(&mut text), span.style));
                    }
                    wrapped.push(Line::from(std::mem::take(&mut output_spans)));
                    output_width = 0;
                }
                text.push(character);
                output_width = output_width.saturating_add(character_width);
            }
            if !text.is_empty() {
                output_spans.push(Span::styled(text, span.style));
            }
        }
        wrapped.push(Line::from(output_spans));
    }
    wrapped
}

fn push_prefixed_lines(
    output: &mut Vec<Line<'static>>,
    label: &str,
    content: &str,
    label_style: Style,
) {
    let sanitized = sanitize_terminal(content);
    let sanitized = truncate_for_ui(&sanitized, 8_000);
    let mut source_lines = sanitized.lines();
    let first = source_lines.next().unwrap_or_default().to_owned();
    output.push(Line::from(vec![
        Span::styled(format!("{label}: "), label_style),
        Span::raw(first),
    ]));
    output.extend(source_lines.map(|line| Line::from(format!("  {line}"))));
}

fn render_modal(frame: &mut Frame<'_>, modal: &Modal, scroll: u16) {
    let area = centered_rect(76, 60, frame.area());
    frame.render_widget(Clear, area);
    let (title, footer, invocation, color) = match modal {
        Modal::Approval(invocation) => (
            " Tool approval ",
            "Up/Down scroll · Y approve · N/Esc deny",
            invocation,
            Color::Yellow,
        ),
        Modal::Recovery(invocation) => (
            " Interrupted tool ",
            "Up/Down scroll · F mark failed and continue · Q quit",
            invocation,
            Color::Red,
        ),
    };
    let arguments = serde_json::to_string_pretty(&invocation.call.arguments)
        .unwrap_or_else(|_| "<invalid arguments>".to_owned());
    let warning = match modal {
        Modal::Approval(_) if invocation.call.name == "run_command" => {
            "WARNING: this starts a host process. Workspace cwd is not an OS sandbox.\n\n"
        }
        Modal::Recovery(_) => {
            "The prior process may have produced side effects. Ikaros will never replay it automatically.\n\n"
        }
        _ => "",
    };
    let text = format!(
        "Tool: {}\nInvocation: {}\nProvider call: {}\n\n{}Arguments:\n{}\n\n{}",
        invocation.call.name, invocation.id, invocation.call.id, warning, arguments, footer
    );
    let widget = Paragraph::new(sanitize_terminal(&text))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color))
                .title(title),
        )
        .alignment(Alignment::Left)
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn truncate_for_ui(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("\n[display truncated]");
    truncated
}

struct TerminalSession;

impl TerminalSession {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable terminal raw mode")?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error).context("failed to enter alternate screen");
        }
        Ok(Self)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = io::stdout().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::{Modal, UiApp, render};
    use crate::domain::{Session, ToolCall, ToolInvocation};
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;

    #[test]
    fn approval_modal_renders_exact_tool() {
        let mut app = UiApp {
            session: Session {
                id: "12345678-0000".to_owned(),
                workspace: "workspace".to_owned(),
                model: "model".to_owned(),
                created_at: 0,
                updated_at: 0,
            },
            transcript: Vec::new(),
            input: String::new(),
            status: "approval required".to_owned(),
            notice: None,
            modal: None,
            modal_scroll: 0,
            should_quit: false,
        };
        app.modal = Some(Modal::Approval(ToolInvocation {
            id: "invocation-1".to_owned(),
            call: ToolCall {
                id: "call-1".to_owned(),
                name: "write_file".to_owned(),
                arguments: json!({"path": "result.txt", "content": "done"}),
            },
        }));
        let backend = TestBackend::new(90, 28);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(frame, &app)).expect("draw");
        let buffer = terminal.backend().buffer();
        let rendered = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Tool approval"));
        assert!(rendered.contains("write_file"));
        assert!(rendered.contains("result.txt"));
    }

    #[test]
    fn wrapped_transcript_scrolls_to_the_latest_text() {
        let app = UiApp {
            session: Session {
                id: "12345678-0000".to_owned(),
                workspace: "workspace".to_owned(),
                model: "model".to_owned(),
                created_at: 0,
                updated_at: 0,
            },
            transcript: vec![crate::domain::Message::assistant(
                Some(format!("{}TAIL_MARKER", "x".repeat(22 * 20))),
                Vec::new(),
            )],
            input: String::new(),
            status: "ready".to_owned(),
            notice: None,
            modal: None,
            modal_scroll: 0,
            should_quit: false,
        };
        let backend = TestBackend::new(24, 10);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(frame, &app)).expect("draw");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("TAIL_MARKER"));
    }
}

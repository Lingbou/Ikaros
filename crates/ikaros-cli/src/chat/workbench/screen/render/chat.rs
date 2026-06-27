// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn render_chat_workbench_with_state(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
    height: usize,
) -> String {
    let width = width.max(40);
    let height = height.max(10);
    let mut lines = Vec::with_capacity(height);
    for line in chat_header_text(screen, width.saturating_sub(4))
        .lines()
        .take(2)
    {
        lines.push(framed_line(line, width));
    }
    lines.push(separator(width));
    let composer_height = usize::from(chat_composer_height(screen, state, height as u16));
    let body_height = height.saturating_sub(5 + composer_height);
    let body_lines =
        chat_text_fallback_body_lines(screen, state, width.saturating_sub(4), body_height);
    for line in body_lines.into_iter().take(body_height) {
        lines.push(framed_line(&line, width));
    }
    while lines.len() < 3 + body_height {
        lines.push(framed_line("", width));
    }
    lines.push(separator(width));
    for line in chat_composer_text(screen, state, width.saturating_sub(4), true)
        .lines()
        .take(composer_height)
    {
        lines.push(framed_line(line, width));
    }
    lines.push(framed_line(
        &chat_footer_text(screen, state, width.saturating_sub(4)),
        width,
    ));
    lines.push(separator(width));
    lines.truncate(height);
    while lines.len() < height {
        lines.insert(lines.len().saturating_sub(1), framed_line("", width));
    }
    lines.join("\n") + "\n"
}

pub(in crate::chat::workbench::screen) fn chat_header_text(
    screen: &WorkbenchScreen,
    width: usize,
) -> String {
    let model = chat_model_label(screen);
    let directory = compact_path_label(&chat_directory_label(screen), width.saturating_sub(12));
    format!(
        "Ikaros\n{}",
        truncate_terminal_text(&format!("{model}  {directory}"), width)
    )
}

pub(in crate::chat::workbench::screen) fn chat_text_fallback_body_lines(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
    height: usize,
) -> Vec<String> {
    if state.command_palette_open && screen_modal_cell(screen).is_none() {
        let mut lines = vec!["Command Palette".into(), String::new()];
        lines.extend(
            command_palette_overlay_text(state)
                .lines()
                .map(str::to_owned),
        );
        return chat_scroll_window(trim_blank_edges(lines), 0, height);
    }
    chat_surface_visible_plain_lines(screen, state, width, height)
}

pub(in crate::chat::workbench::screen) fn chat_surface_styled_lines(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    chat_surface_visible_plain_lines(screen, state, width, height)
        .into_iter()
        .map(chat_styled_line)
        .collect()
}

pub(in crate::chat::workbench::screen) fn chat_model_label(screen: &WorkbenchScreen) -> String {
    find_cell(screen, |cell| cell.title == "model")
        .and_then(|cell| extract_token_after(&cell.detail, "model="))
        .unwrap_or_else(|| "unknown".into())
}

pub(in crate::chat::workbench::screen) fn chat_directory_label(screen: &WorkbenchScreen) -> String {
    find_cell(screen, |cell| cell.title == "workspace")
        .and_then(|cell| extract_assignment_span(&cell.detail, "path=", &[]))
        .unwrap_or_else(|| ".".into())
}

pub(in crate::chat::workbench::screen) fn chat_surface_visible_plain_lines(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
    height: usize,
) -> Vec<String> {
    let lines = trim_blank_edges(chat_surface_plain_lines(screen, width));
    chat_scroll_window(lines, state.scroll_for(WorkbenchScreenPanel::Main), height)
}

pub(in crate::chat::workbench::screen) fn chat_surface_max_scroll(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
    height: usize,
) -> usize {
    let lines = trim_blank_edges(chat_surface_plain_lines(screen, width));
    lines
        .len()
        .saturating_sub(chat_surface_body_height(screen, state, height))
}

pub(in crate::chat::workbench::screen) fn chat_surface_body_height(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    height: usize,
) -> usize {
    let composer_height = usize::from(chat_composer_height(screen, state, height as u16));
    height.saturating_sub(3 + composer_height)
}

pub(in crate::chat::workbench::screen) fn chat_scroll_window(
    lines: Vec<String>,
    scroll_from_bottom: usize,
    height: usize,
) -> Vec<String> {
    if height == 0 || lines.is_empty() {
        return Vec::new();
    }
    let max_scroll = lines.len().saturating_sub(height);
    let scroll = scroll_from_bottom.min(max_scroll);
    let end = lines.len().saturating_sub(scroll);
    let start = end.saturating_sub(height);
    lines[start..end].to_vec()
}

pub(in crate::chat::workbench::screen) fn chat_surface_plain_lines(
    screen: &WorkbenchScreen,
    width: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    lines.extend(chat_conversation_lines(screen, width));
    if let Some(progress) = chat_progress_line(screen) {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(progress);
        lines.push(String::new());
    }
    lines.extend(chat_notice_lines(screen, width));
    if lines.is_empty() {
        lines.push("Ask Ikaros to do anything.".into());
    }
    lines
}

pub(in crate::chat::workbench::screen) fn chat_styled_line(line: String) -> Line<'static> {
    if line.chars().all(|ch| ch == '─') {
        return Line::from(Span::styled(
            line,
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    if let Some(rest) = line.strip_prefix("> ") {
        return Line::from(vec![
            Span::styled(
                ">",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {rest}")),
        ]);
    }
    if line.starts_with("Working ") || line.starts_with("Working(") {
        return Line::from(Span::styled(line, Style::default().fg(Color::Yellow)));
    }
    if line.starts_with("Turn failed.") {
        return Line::from(Span::styled(line, Style::default().fg(Color::Red)));
    }

    let leading_spaces = line.chars().take_while(|ch| *ch == ' ').count();
    let trimmed = line[leading_spaces..].to_owned();
    if let Some(rest) = trimmed.strip_prefix("• ") {
        return Line::from(vec![
            Span::raw(" ".repeat(leading_spaces)),
            Span::styled("•", Style::default().fg(Color::Green)),
            Span::raw(format!(" {rest}")),
        ]);
    }

    Line::from(line)
}

pub(in crate::chat::workbench::screen) fn chat_notice_lines(
    screen: &WorkbenchScreen,
    width: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    let progress_status = find_cell(screen, |cell| cell.title == "progress")
        .and_then(|cell| extract_token_after(&cell.detail, "status="))
        .unwrap_or_else(|| "idle".into());
    for cell in screen
        .main
        .iter()
        .chain(screen.side.iter())
        .filter(|cell| cell.title.starts_with("notice "))
        .filter(|cell| chat_notice_visible(cell, &progress_status))
        .take(4)
    {
        let prefix = match cell.kind {
            WorkbenchCellKind::Error => "Error:",
            WorkbenchCellKind::Approval => "Approval:",
            _ => "Notice:",
        };
        let title = cell.title.trim_start_matches("notice ").trim();
        let detail = notice_human_detail(cell);
        lines.extend(wrap(
            &format!("{prefix} {} {}", terminal_inline(title), detail),
            width,
        ));
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub(in crate::chat::workbench::screen) fn chat_notice_visible(
    cell: &WorkbenchCell,
    progress_status: &str,
) -> bool {
    let kind = notice_kind(cell);
    if matches!(kind.as_deref(), Some("progress")) {
        return false;
    }
    let title = cell.title.trim_start_matches("notice ").trim();
    if matches!(
        title,
        "chat_turn"
            | "command palette"
            | "command executed"
            | "command routed"
            | "model"
            | "screen open selected"
            | "pending input requeued"
            | "pending inputs restored"
    ) {
        return false;
    }
    let detail = notice_detail_text(cell);
    let detail_lower = detail.to_ascii_lowercase();
    if detail_lower == "queue is empty"
        || detail_lower == "pending input queue is empty"
        || detail_lower.contains("pending_input_run: pending_inputs=0 status=empty")
    {
        return false;
    }
    if title == "pending input"
        && (matches!(detail.as_str(), "queue is empty" | "queue drain completed")
            || detail.starts_with("running queued input "))
    {
        return false;
    }
    if progress_status == "failed"
        && (title == "chat turn failed" || matches!(kind.as_deref(), Some("error")))
    {
        return false;
    }
    true
}

pub(in crate::chat::workbench::screen) fn notice_kind(cell: &WorkbenchCell) -> Option<String> {
    extract_token_after(&cell.detail, "notice_kind=")
}

pub(in crate::chat::workbench::screen) fn notice_detail_text(cell: &WorkbenchCell) -> String {
    extract_assignment_span(&cell.detail, "detail=", &[])
        .or_else(|| extract_assignment_span(&cell.detail, "message=", &[" command=", " trace="]))
        .unwrap_or_else(|| human_cell_detail(cell))
}

pub(in crate::chat::workbench::screen) fn notice_human_detail(cell: &WorkbenchCell) -> String {
    let detail = notice_detail_text(cell);
    if cell.kind == WorkbenchCellKind::Error {
        return human_error_summary(&detail);
    }
    if detail.contains("source=") || detail.contains("reason=") || detail.contains("status=") {
        return human_cell_detail(cell);
    }
    truncate_terminal_text(&detail, 120)
}

pub(in crate::chat::workbench::screen) fn chat_conversation_lines(
    screen: &WorkbenchScreen,
    width: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    let conversation = screen
        .main
        .iter()
        .filter(|cell| {
            cell.title.starts_with("user turn=") || cell.title.starts_with("assistant turn=")
        })
        .collect::<Vec<_>>();
    for cell in conversation {
        if cell.title.starts_with("user turn=") && !lines.is_empty() {
            lines.push(chat_turn_separator(width));
            lines.push(String::new());
        }
        let prefix = if cell.title.starts_with("user ") {
            ">"
        } else {
            ""
        };
        lines.extend(chat_message_lines(prefix, &cell.detail, width));
        lines.push(String::new());
    }
    lines
}

pub(in crate::chat::workbench::screen) fn chat_message_lines(
    prefix: &str,
    detail: &str,
    width: usize,
) -> Vec<String> {
    let body_width = width.saturating_sub(2).max(8);
    let wrapped = wrap(&render_terminal_markdown(detail), body_width);
    let mut lines = Vec::new();
    let mut first_content_line = true;
    for line in wrapped {
        if line.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        if first_content_line && !prefix.is_empty() {
            lines.push(format!("{prefix} {line}"));
            first_content_line = false;
        } else if first_content_line {
            lines.push(line);
            first_content_line = false;
        } else {
            lines.push(format!("  {line}"));
        }
    }
    if lines.is_empty() {
        lines.push(prefix.to_owned());
    }
    lines
}

pub(in crate::chat::workbench::screen) fn chat_turn_separator(width: usize) -> String {
    "─".repeat(width.max(16))
}

pub(in crate::chat::workbench::screen) fn chat_progress_line(
    screen: &WorkbenchScreen,
) -> Option<String> {
    let progress = find_cell(screen, |cell| cell.title == "progress")?;
    let status = extract_token_after(&progress.detail, "status=").unwrap_or_else(|| "idle".into());
    match status.as_str() {
        "running" => {
            let elapsed = extract_token_after(&progress.detail, "elapsed_ms=")
                .and_then(|value| value.parse::<u128>().ok())
                .map(format_progress_elapsed_ms)
                .unwrap_or_else(|| "now".into());
            Some(format!("Working ({elapsed} - esc to interrupt)"))
        }
        "approval_pending" => Some("Waiting for approval. Use Alt+A or Alt+D.".into()),
        "failed" => Some(format!(
            "Turn failed. {}",
            human_error_summary(
                &extract_assignment_span(
                    &progress.detail,
                    "detail=",
                    &[" command=", " budget=", " trace="]
                )
                .unwrap_or_else(|| "Press F5 for recovery actions.".into())
            )
        )),
        _ => None,
    }
}

fn format_progress_elapsed_ms(value: u128) -> String {
    let seconds = value / 1000;
    if seconds < 10 {
        let tenths = (value % 1000) / 100;
        format!("{seconds}.{tenths}s")
    } else {
        format!("{seconds}s")
    }
}

pub(in crate::chat::workbench::screen) fn chat_composer_height(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    available_height: u16,
) -> u16 {
    let popup = screen_input_popup_json(screen, state);
    let desired = 2 + chat_inline_popup_height(&popup);
    desired.min(available_height.saturating_sub(4).max(2))
}

pub(in crate::chat::workbench::screen) fn chat_composer_text(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
    include_popup: bool,
) -> String {
    let mut lines = Vec::new();
    if include_popup {
        let popup = screen_input_popup_json(screen, state);
        let popup_lines = chat_popup_lines(&popup, width);
        if !popup_lines.is_empty() {
            lines.push(chat_popup_title(&popup).into());
            lines.extend(popup_lines);
            lines.push(String::new());
        }
    }
    lines.push(chat_composer_prompt_line(screen, width));
    lines.join("\n")
}

pub(in crate::chat::workbench::screen) fn draw_chat_composer(
    frame: &mut Frame<'_>,
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    area: Rect,
) {
    let popup = screen_input_popup_json(screen, state);
    let popup_height = chat_inline_popup_height(&popup).min(area.height.saturating_sub(2));
    if popup_height > 0 {
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(popup_height), Constraint::Min(2)])
            .split(area);
        frame.render_widget(
            Paragraph::new(chat_popup_lines(&popup, vertical[0].width as usize).join("\n"))
                .block(
                    Block::default()
                        .title(chat_popup_title(&popup))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            vertical[0],
        );
        frame.render_widget(
            Paragraph::new(chat_composer_prompt_line(
                screen,
                vertical[1].width as usize,
            ))
            .wrap(Wrap { trim: true }),
            vertical[1],
        );
        return;
    }
    frame.render_widget(
        Paragraph::new(chat_composer_text(
            screen,
            state,
            area.width as usize,
            false,
        ))
        .wrap(Wrap { trim: true }),
        area,
    );
}

pub(in crate::chat::workbench::screen) fn chat_composer_prompt_line(
    screen: &WorkbenchScreen,
    width: usize,
) -> String {
    let input_view = extract_assignment_span(&screen.input_hint, "view=", &[" undo="])
        .unwrap_or_else(|| String::new());
    let input_view = terminal_inline(&input_view);
    let display = if input_view.trim().is_empty() {
        "Ask Ikaros to do anything".into()
    } else {
        input_view
    };
    truncate_terminal_text(&format!("> {display}"), width)
}

pub(in crate::chat::workbench::screen) fn chat_inline_popup_height(
    popup: &serde_json::Value,
) -> u16 {
    match chat_popup_kind(popup) {
        "command_completion" => {
            let count = chat_popup_items(popup).len().clamp(1, 6);
            count as u16 + 2
        }
        "history_search" => 3,
        _ => 0,
    }
}

pub(in crate::chat::workbench::screen) fn chat_popup_title(
    popup: &serde_json::Value,
) -> &'static str {
    match chat_popup_kind(popup) {
        "history_search" => "History",
        "command_completion" => "Slash Commands",
        _ => "Picker",
    }
}

pub(in crate::chat::workbench::screen) fn chat_popup_kind(popup: &serde_json::Value) -> &str {
    popup
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none")
}

pub(in crate::chat::workbench::screen) fn chat_popup_items(
    popup: &serde_json::Value,
) -> Vec<serde_json::Value> {
    let items_key = match chat_popup_kind(popup) {
        "command_completion" => "completion_items",
        _ => return Vec::new(),
    };
    popup
        .get(items_key)
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(in crate::chat::workbench::screen) fn chat_popup_lines(
    popup: &serde_json::Value,
    width: usize,
) -> Vec<String> {
    match chat_popup_kind(popup) {
        "command_completion" => chat_command_completion_lines(popup, width),
        "history_search" => vec![truncate_terminal_text(
            "Search history. Enter accepts, Esc closes.",
            width,
        )],
        _ => Vec::new(),
    }
}

pub(in crate::chat::workbench::screen) fn chat_command_completion_lines(
    popup: &serde_json::Value,
    width: usize,
) -> Vec<String> {
    let items = chat_popup_items(popup);
    if items.is_empty() {
        return vec![truncate_terminal_text("No matching slash commands.", width)];
    }
    items
        .iter()
        .take(6)
        .map(|item| chat_command_completion_item_line(item, width))
        .collect()
}

pub(in crate::chat::workbench::screen) fn chat_command_completion_item_line(
    item: &serde_json::Value,
    width: usize,
) -> String {
    let marker = if item
        .get("selected")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        ">"
    } else {
        " "
    };
    let command = json_string(item, "command", "none");
    let summary = json_string(item, "summary", "none");
    truncate_terminal_text(&format!("{marker} {command:<18} {summary}"), width)
}

pub(in crate::chat::workbench::screen) fn chat_footer_text(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
) -> String {
    let model = chat_model_label(screen);
    let directory = chat_directory_label(screen);
    let scroll = state.scroll_for(WorkbenchScreenPanel::Main);
    let scroll_label = match scroll {
        0 => String::new(),
        usize::MAX => " - oldest messages".into(),
        scroll => format!(" - {scroll} lines above latest"),
    };
    truncate_terminal_text(
        &format!(
            "{} - {}{}",
            terminal_inline(&model),
            compact_path_label(&directory, width.saturating_sub(model.chars().count() + 3)),
            scroll_label,
        ),
        width,
    )
}

pub(in crate::chat::workbench::screen) fn compact_path_label(path: &str, width: usize) -> String {
    let path = terminal_inline(path);
    let count = path.chars().count();
    if count <= width || width < 12 {
        return truncate_terminal_text(&path, width);
    }
    let tail_len = width.saturating_sub(3);
    let tail = path
        .chars()
        .rev()
        .take(tail_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("...{tail}")
}

pub(in crate::chat::workbench::screen) fn human_error_summary(detail: &str) -> String {
    let detail = terminal_inline(detail);
    let lower = detail.to_ascii_lowercase();
    if lower.contains("tools.function.parameters") || lower.contains("tool schema") {
        return "Provider rejected the tool schema. Try again after compatibility is fixed.".into();
    }
    if lower.contains("http 400") || lower.contains("bad request") {
        return "Provider rejected the request (HTTP 400).".into();
    }
    if lower.contains("rate limit") || lower.contains("429") {
        return "Provider rate limit hit. Try again later or switch model.".into();
    }
    if lower.contains("timeout") {
        return "Provider request timed out. Try again or switch model.".into();
    }
    if lower.contains("dns") || lower.contains("network") {
        return "Network/provider connection failed.".into();
    }
    short_status_detail(&detail)
}

pub(in crate::chat::workbench::screen) fn trim_blank_edges(mut lines: Vec<String>) -> Vec<String> {
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

pub(in crate::chat::workbench::screen) fn short_status_detail(detail: &str) -> String {
    let detail = terminal_inline(detail);
    if detail.trim().is_empty() || detail == "none" {
        return "Press F5 for actions.".into();
    }
    truncate_terminal_text(&detail, 120)
}

pub(in crate::chat::workbench::screen) fn truncate_terminal_text(
    input: &str,
    max_chars: usize,
) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut output = String::new();
    for (index, ch) in terminal_inline(input).chars().enumerate() {
        if index >= max_chars {
            if max_chars > 3 {
                output.push_str("...");
            }
            return output;
        }
        output.push(ch);
    }
    output
}

// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub fn render_fullscreen_workbench_with_state(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
    height: usize,
) -> String {
    if !state.raw_mode() {
        return render_chat_workbench_with_state(screen, state, width, height);
    }
    let width = width.max(40);
    let height = height.max(10);
    let mut lines = Vec::with_capacity(height);
    let modal = screen_modal_cell(screen);

    lines.push(border_title(&screen.title, width));
    lines.push(framed_line(
        &format!(
            "{} {}",
            panel_title("Status", WorkbenchScreenPanel::Status, state),
            inline_cell_summary(screen.status.iter(), width.saturating_sub(10))
        ),
        width,
    ));
    if let Some(modal) = modal {
        lines.push(framed_line(&screen_modal_summary(modal), width));
    }
    if state.command_palette_open {
        lines.push(framed_line(
            &command_palette_summary_line(screen, state),
            width,
        ));
    }
    lines.push(separator(width));
    lines.push(three_column_row(
        panel_title("Timeline", WorkbenchScreenPanel::Timeline, state),
        panel_title("Main", WorkbenchScreenPanel::Main, state),
        panel_title(side_panel_title(screen), WorkbenchScreenPanel::Side, state),
        width,
    ));

    let extra_rows = usize::from(modal.is_some()) + usize::from(state.command_palette_open);
    let body_height = height.saturating_sub(8 + extra_rows);
    let (left_width, main_width, side_width) = column_widths(width);
    let timeline = panel_lines(
        &screen.timeline,
        left_width,
        state.scroll_for(WorkbenchScreenPanel::Timeline),
        Some(state.selection_for(WorkbenchScreenPanel::Timeline)),
        state.raw_mode(),
    );
    let main = panel_lines(
        &screen.main,
        main_width,
        state.scroll_for(WorkbenchScreenPanel::Main),
        Some(state.selection_for(WorkbenchScreenPanel::Main)),
        state.raw_mode(),
    );
    let main = if body_height >= 10 {
        let mut lines = main_dashboard_lines(screen, main_width, state.raw_mode());
        lines.extend(main);
        lines
    } else {
        main
    };
    let side = panel_lines(
        &screen.side,
        side_width,
        state.scroll_for(WorkbenchScreenPanel::Side),
        Some(state.side_selection),
        state.raw_mode(),
    );
    for index in 0..body_height {
        lines.push(three_column_row(
            timeline.get(index).map(String::as_str).unwrap_or(""),
            main.get(index).map(String::as_str).unwrap_or(""),
            side.get(index).map(String::as_str).unwrap_or(""),
            width,
        ));
    }

    lines.push(separator(width));
    lines.push(framed_line(
        &selected_cell_detail_line(screen, state),
        width,
    ));
    lines.push(framed_line(
        &format!(
            "Input {} {}",
            terminal_inline(&screen.input_hint),
            terminal_inline(&screen.footer),
        ),
        width,
    ));
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

pub fn draw_tui_workbench_frame(
    frame: &mut Frame<'_>,
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) {
    if !state.raw_mode() {
        draw_chat_tui_frame(frame, screen, state);
        return;
    }
    let area = frame.area();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(8),
        ])
        .split(area);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(45),
            Constraint::Percentage(30),
        ])
        .split(vertical[1]);
    let title = format!(
        "{} | {} | {}",
        terminal_inline(&screen.title),
        inline_cell_summary(screen.status.iter(), area.width.saturating_sub(4) as usize),
        evidence_attention_summary(screen),
    );
    let title = screen_modal_cell(screen)
        .map(|modal| format!("{title} | {}", screen_modal_summary(modal)))
        .unwrap_or(title);
    frame.render_widget(
        Paragraph::new(title)
            .block(
                Block::default()
                    .title(panel_title("Ikaros", WorkbenchScreenPanel::Status, state).into_owned())
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: true }),
        vertical[0],
    );
    frame.render_widget(
        panel_paragraph("Timeline", WorkbenchScreenPanel::Timeline, screen, state),
        body[0],
    );
    frame.render_widget(
        panel_paragraph("Main", WorkbenchScreenPanel::Main, screen, state),
        body[1],
    );
    frame.render_widget(
        panel_paragraph(
            side_panel_title(screen),
            WorkbenchScreenPanel::Side,
            screen,
            state,
        ),
        body[2],
    );
    frame.render_widget(
        Paragraph::new(selected_cell_detail_line(screen, state))
            .block(Block::default().title("Selected").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        vertical[2],
    );
    frame.render_widget(
        Paragraph::new(bottom_pane_text(screen, state))
            .block(Block::default().title("Input").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        vertical[3],
    );
    draw_command_palette_overlay(frame, screen, state);
    draw_approval_modal_overlay(frame, screen);
}

pub(crate) fn draw_chat_tui_frame(
    frame: &mut Frame<'_>,
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) {
    let area = frame.area();
    let composer_height = chat_composer_height(screen, state, area.height);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(composer_height),
            Constraint::Length(1),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(chat_header_text(screen, vertical[0].width as usize))
            .style(Style::default().add_modifier(Modifier::BOLD))
            .wrap(Wrap { trim: true }),
        vertical[0],
    );
    frame.render_widget(
        Paragraph::new(chat_surface_styled_lines(
            screen,
            state,
            vertical[1].width as usize,
            vertical[1].height as usize,
        ))
        .wrap(Wrap { trim: false }),
        vertical[1],
    );
    draw_chat_composer(frame, screen, state, vertical[2]);
    frame.render_widget(
        Paragraph::new(chat_footer_text(screen, state, vertical[3].width as usize))
            .wrap(Wrap { trim: true }),
        vertical[3],
    );
    draw_command_palette_overlay(frame, screen, state);
    draw_approval_modal_overlay(frame, screen);
}

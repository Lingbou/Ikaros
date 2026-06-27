// SPDX-License-Identifier: GPL-3.0-only

use super::{
    WorkbenchScreen, WorkbenchScreenAction, WorkbenchScreenApprovalAction,
    WorkbenchScreenContinuationAction, WorkbenchScreenInputAction, WorkbenchScreenOpenAction,
    WorkbenchScreenPanel, WorkbenchScreenState, action_menu_queue_items_json,
    apply_workbench_screen_key_event, apply_workbench_screen_key_event_with_view,
    apply_workbench_screen_mouse_event, command_palette_overlay_json, parse_workbench_screen_state,
    render_fullscreen_terminal_frame, render_fullscreen_workbench_with_state,
    render_tui_workbench_snapshot, screen_json_line, screen_queue_panel_json,
    screen_selected_actions_json_line, screen_selected_actions_line, screen_selected_cell_line,
    screen_selected_primary_action,
};
use crate::chat::workbench::{WorkbenchCell, WorkbenchCellKind};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

fn cell(kind: WorkbenchCellKind, title: &str, detail: &str) -> WorkbenchCell {
    WorkbenchCell {
        kind,
        title: title.to_owned(),
        detail: detail.to_owned(),
    }
}

fn raw_state() -> WorkbenchScreenState {
    parse_workbench_screen_state(&["--raw"]).expect("raw screen state")
}

#[test]
fn screen_navigation_marks_focus_and_applies_panel_scroll() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![cell(WorkbenchCellKind::Model, "model", "ok")],
        timeline: vec![cell(WorkbenchCellKind::Session, "old", "hidden")],
        main: vec![
            cell(WorkbenchCellKind::Context, "top", "hidden"),
            cell(WorkbenchCellKind::Coding, "visible", "kept"),
        ],
        side: vec![cell(WorkbenchCellKind::Approval, "approval", "pending")],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let mut state = raw_state();

    state.apply(WorkbenchScreenAction::FocusNext);
    state.apply(WorkbenchScreenAction::ScrollDown);

    assert_eq!(state.focused_panel(), WorkbenchScreenPanel::Main);
    let frame = render_fullscreen_workbench_with_state(&screen, &state, 76, 14);

    assert!(frame.contains("Main*"));
    assert!(frame.contains("visible"));
    assert!(!frame.contains("top"));
    assert!(frame.contains("focus=main"));
    assert!(frame.contains("scroll=main:1"));
    assert!(state.footer_summary().contains("pgup/pgdn/home"));
    for line in frame.lines() {
        assert!(
            line.chars().count() <= 76,
            "frame line exceeds width: {line}"
        );
    }
    assert_eq!(frame.lines().count(), 14);
}

#[test]
fn fullscreen_terminal_frame_wraps_rendered_workbench_in_terminal_envelope() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![cell(WorkbenchCellKind::Model, "model", "ok")],
        timeline: vec![cell(WorkbenchCellKind::Session, "turn", "ok")],
        main: vec![cell(WorkbenchCellKind::Context, "context", "ok")],
        side: vec![cell(WorkbenchCellKind::Approval, "approval", "pending")],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let frame = render_fullscreen_terminal_frame(&screen, &raw_state(), 80, 12);

    assert!(frame.starts_with("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H"));
    assert!(frame.contains("Ikaros Workbench"));
    assert!(frame.contains("model"));
    assert!(frame.contains("ok"));
    assert!(frame.ends_with("\x1b[?25h\x1b[?1049l"));
    assert_eq!(frame.matches("\x1b[?1049h").count(), 1);
    assert_eq!(frame.matches("\x1b[?1049l").count(), 1);
}

#[test]
fn screen_panel_preserves_rendered_markdown_line_structure() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![cell(WorkbenchCellKind::Model, "model delta", "ok")],
        main: vec![cell(
            WorkbenchCellKind::Model,
            "assistant output",
            "[code rust]\n  let value = 1;\n[/code]\n[table]\n  File | Status\n[/table]",
        )],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };

    let frame = render_fullscreen_workbench_with_state(&screen, &raw_state(), 88, 16);

    assert!(frame.contains("[code rust]"));
    assert!(frame.contains("let value = 1;"));
    assert!(frame.contains("[table]"));
    assert!(frame.contains("File | Status"));
    assert!(!frame.contains("rust]_"));
    assert!(!frame.contains("[/code]_[table]"));
}

#[test]
fn ratatui_default_snapshot_renders_clean_chat_surface_without_machine_noise() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![
            cell(
                WorkbenchCellKind::Model,
                "model",
                "provider=mock model=mock-chat",
            ),
            cell(
                WorkbenchCellKind::Session,
                "workspace",
                "path=/tmp/ikaros-workspace",
            ),
        ],
        timeline: vec![cell(
            WorkbenchCellKind::Session,
            "turn=turn-one",
            "status=running",
        )],
        main: vec![
            cell(WorkbenchCellKind::Session, "user turn=turn-one", "hello"),
            cell(
                WorkbenchCellKind::Model,
                "assistant turn=turn-one",
                "Hello. I can help with code.",
            ),
            cell(
                WorkbenchCellKind::Context,
                "context budget",
                "used=42 secret=sk-secret-value",
            ),
        ],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "input_state: view= undo=0".into(),
    };
    let state = WorkbenchScreenState::default();

    let snapshot =
        render_tui_workbench_snapshot(&screen, &state, 96, 24).expect("ratatui snapshot");

    assert!(snapshot.contains("Ikaros"));
    assert!(snapshot.contains("mock-chat"));
    assert!(snapshot.contains("/tmp/ikaros-workspace"));
    assert!(snapshot.contains("> hello"));
    assert!(snapshot.contains("Hello. I can help with code."));
    assert!(snapshot.contains("Ask Ikaros to do anything"));
    assert!(!snapshot.contains("Timeline"));
    assert!(!snapshot.contains("Approvals / Queue"));
    assert!(!snapshot.contains("actions selector="));
    assert!(!snapshot.contains("selected panel="));
    assert!(!snapshot.contains("surface=composer"));
    assert!(!snapshot.contains("provider=mock"));
    assert!(!snapshot.contains("used=42"));
    assert!(!snapshot.contains("sk-secret-value"));
    assert!(!snapshot.contains("secret=sk-"));
}

#[test]
fn ratatui_default_snapshot_filters_internal_turn_notices() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![
            cell(
                WorkbenchCellKind::Model,
                "model",
                "provider=moonshot model=kimi-k2.6",
            ),
            cell(
                WorkbenchCellKind::Session,
                "workspace",
                "path=/tmp/ikaros-workspace",
            ),
            cell(
                WorkbenchCellKind::Continuation,
                "progress",
                "status=failed detail=model provider returned HTTP 400: {\"error\":{\"message\":\"Invalid request: tools.function.parameters is not a valid moonshot favored json schema\"}}",
            ),
        ],
        timeline: vec![],
        main: vec![
            cell(WorkbenchCellKind::Session, "user turn=turn-one", "你好"),
            cell(
                WorkbenchCellKind::Continuation,
                "notice chat_turn",
                "notice_kind=progress detail=status=running elapsed_ms=0 detail=你好",
            ),
            cell(
                WorkbenchCellKind::Error,
                "notice chat turn failed",
                "notice_kind=error detail=model provider returned HTTP 400: {\"error\":{\"message\":\"Invalid request: tools.function.parameters is not a valid moonshot favored json schema\"}}",
            ),
            cell(
                WorkbenchCellKind::Continuation,
                "notice pending input requeued",
                "notice_kind=continuation detail=source=interactive reason=turn_failure",
            ),
            cell(
                WorkbenchCellKind::Continuation,
                "notice pending input",
                "notice_kind=continuation detail=queue is empty",
            ),
            cell(
                WorkbenchCellKind::Session,
                "notice command executed",
                "notice_kind=info detail=command=/model",
            ),
            cell(
                WorkbenchCellKind::Session,
                "notice command routed",
                "notice_kind=info detail=/tools is shown through the fullscreen workbench instead of raw terminal output",
            ),
            cell(
                WorkbenchCellKind::Session,
                "notice command palette",
                "notice_kind=info detail=opened slash command picker",
            ),
            cell(
                WorkbenchCellKind::Session,
                "notice model",
                "notice_kind=info detail=model status refreshed in the workbench",
            ),
        ],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "input_state: view= undo=0".into(),
    };
    let state = WorkbenchScreenState::default();

    let snapshot =
        render_tui_workbench_snapshot(&screen, &state, 110, 24).expect("ratatui snapshot");

    assert!(
        snapshot.contains("> 你好") || snapshot.contains("> 你 好"),
        "snapshot:\n{snapshot}"
    );
    assert!(snapshot.contains("Turn failed. Provider rejected the tool schema."));
    assert!(!snapshot.contains("Tip:"));
    assert!(!snapshot.contains("notice chat_turn"));
    assert!(!snapshot.contains("chat_turn status=running"));
    assert!(!snapshot.contains("pending input requeued"));
    assert!(!snapshot.contains("pending input queue is empty"));
    assert!(!snapshot.contains("command=/model"));
    assert!(!snapshot.contains("raw terminal output"));
    assert!(!snapshot.contains("opened slash command picker"));
    assert!(!snapshot.contains("model status refreshed"));
    assert!(!snapshot.contains("tools.function.parameters"));
    assert!(!snapshot.contains("Invalid request:"));
}

#[test]
fn ratatui_default_snapshot_preserves_assistant_markdown_structure() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![
            cell(
                WorkbenchCellKind::Model,
                "model",
                "provider=mock model=mock-chat",
            ),
            cell(
                WorkbenchCellKind::Session,
                "workspace",
                "path=/tmp/ikaros-workspace",
            ),
        ],
        timeline: vec![],
        main: vec![
            cell(
                WorkbenchCellKind::Session,
                "user turn=turn-one",
                "what can you do",
            ),
            cell(
                WorkbenchCellKind::Model,
                "assistant turn=turn-one",
                "### Files and code\n- Browse and edit workspace files\n- Review Git changes\n\n### System\n- Explain state",
            ),
        ],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "input_state: view= undo=0".into(),
    };

    let snapshot =
        render_tui_workbench_snapshot(&screen, &WorkbenchScreenState::default(), 110, 22)
            .expect("ratatui snapshot");

    assert!(snapshot.contains("Files and code"));
    assert!(snapshot.contains("• Browse and edit workspace files"));
    assert!(snapshot.contains("• Review Git changes"));
    assert!(snapshot.contains("System"));
    assert!(!snapshot.contains(" / - Browse"));
    assert!(!snapshot.contains("### Files and code"));
}

#[test]
fn ratatui_default_snapshot_separates_chat_turns() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![
            cell(
                WorkbenchCellKind::Model,
                "model",
                "provider=mock model=mock-chat",
            ),
            cell(
                WorkbenchCellKind::Session,
                "workspace",
                "path=/tmp/ikaros-workspace",
            ),
        ],
        timeline: vec![],
        main: vec![
            cell(WorkbenchCellKind::Session, "user turn=turn-one", "first"),
            cell(WorkbenchCellKind::Model, "assistant turn=turn-one", "done"),
            cell(WorkbenchCellKind::Session, "user turn=turn-two", "second"),
            cell(
                WorkbenchCellKind::Model,
                "assistant turn=turn-two",
                "done again",
            ),
        ],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "input_state: view= undo=0".into(),
    };

    let snapshot =
        render_tui_workbench_snapshot(&screen, &WorkbenchScreenState::default(), 100, 24)
            .expect("ratatui snapshot");

    assert!(snapshot.contains("> first"));
    assert!(snapshot.contains("> second"));
    assert!(snapshot.lines().any(|line| line.contains("────────")));
}

#[test]
fn ratatui_default_snapshot_scrolls_chat_transcript_from_bottom() {
    let mut main = Vec::new();
    for index in 1..=30 {
        main.push(cell(
            WorkbenchCellKind::Session,
            &format!("user turn=turn-{index:02}"),
            &format!("question {index:02}"),
        ));
        main.push(cell(
            WorkbenchCellKind::Model,
            &format!("assistant turn=turn-{index:02}"),
            &format!("answer {index:02}"),
        ));
    }
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![
            cell(
                WorkbenchCellKind::Model,
                "model",
                "provider=mock model=mock-chat",
            ),
            cell(
                WorkbenchCellKind::Session,
                "workspace",
                "path=/tmp/ikaros-workspace",
            ),
        ],
        timeline: vec![],
        main,
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "input_state: view= undo=0".into(),
    };
    let latest = render_tui_workbench_snapshot(&screen, &WorkbenchScreenState::default(), 96, 13)
        .expect("latest snapshot");

    assert!(latest.contains("question 30"));
    assert!(latest.contains("answer 30"));
    assert!(!latest.contains("question 01"));

    let scrolled = WorkbenchScreenState {
        main_scroll: usize::MAX,
        ..Default::default()
    };
    let older = render_tui_workbench_snapshot(&screen, &scrolled, 96, 13).expect("older snapshot");

    assert!(older.contains("question 01"));
    assert!(older.contains("answer 01"));
    assert!(!older.contains("question 30"));
}

#[test]
fn ratatui_default_snapshot_moves_down_from_home_without_jumping_to_bottom() {
    let mut main = Vec::new();
    for index in 1..=30 {
        main.push(cell(
            WorkbenchCellKind::Session,
            &format!("user turn=turn-{index:02}"),
            &format!("question {index:02}"),
        ));
        main.push(cell(
            WorkbenchCellKind::Model,
            &format!("assistant turn=turn-{index:02}"),
            &format!("answer {index:02}"),
        ));
    }
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![
            cell(
                WorkbenchCellKind::Model,
                "model",
                "provider=mock model=mock-chat",
            ),
            cell(
                WorkbenchCellKind::Session,
                "workspace",
                "path=/tmp/ikaros-workspace",
            ),
        ],
        timeline: vec![],
        main,
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "input_state: view= undo=0".into(),
    };
    let mut state = WorkbenchScreenState::default();

    assert!(apply_workbench_screen_key_event_with_view(
        &mut state,
        KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
        Some(&screen),
        96,
        13,
    ));
    assert!(apply_workbench_screen_key_event_with_view(
        &mut state,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        Some(&screen),
        96,
        13,
    ));

    let scrolled =
        render_tui_workbench_snapshot(&screen, &state, 96, 13).expect("scrolled snapshot");

    assert!(state.main_scroll > 0);
    assert!(state.main_scroll < usize::MAX);
    assert!(scrolled.contains("answer 01"));
    assert!(!scrolled.contains("question 30"));
}

#[test]
fn screen_mouse_wheel_scrolls_default_chat_history() {
    let mut state = WorkbenchScreenState::default();
    let scroll_up = MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 4,
        row: 4,
        modifiers: KeyModifiers::NONE,
    };
    let scroll_down = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 4,
        row: 4,
        modifiers: KeyModifiers::NONE,
    };

    assert!(apply_workbench_screen_mouse_event(&mut state, scroll_up));
    assert_eq!(state.main_scroll, 3);
    assert!(apply_workbench_screen_mouse_event(&mut state, scroll_down));
    assert_eq!(state.main_scroll, 0);
}

#[test]
fn ratatui_default_snapshot_renders_slash_completion_as_picker() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![
            cell(
                WorkbenchCellKind::Model,
                "model",
                "provider=mock model=mock-chat",
            ),
            cell(
                WorkbenchCellKind::Session,
                "workspace",
                "path=/tmp/ikaros-workspace",
            ),
        ],
        timeline: vec![],
        main: vec![
            cell(
                WorkbenchCellKind::Session,
                "command completion",
                "query=/a candidates=2 selected=/agent tab=cycle enter=run command=/commands /a",
            ),
            cell(
                WorkbenchCellKind::Session,
                "command /agent",
                "usage=/agent args=optional effect=config-mutation command=/agent summary=switch active agent profile or instance",
            ),
            cell(
                WorkbenchCellKind::Session,
                "command /approval",
                "usage=/approval args=optional effect=approval-decision command=/approval summary=show or resolve pending approvals",
            ),
        ],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "input_state: view=/a undo=0".into(),
    };
    let state = WorkbenchScreenState::default();

    let snapshot =
        render_tui_workbench_snapshot(&screen, &state, 96, 22).expect("ratatui snapshot");

    assert!(snapshot.contains("Slash Commands"));
    assert!(snapshot.contains("> /agent"));
    assert!(snapshot.contains("/approval"));
    assert!(snapshot.contains("> /a"));
    assert!(!snapshot.contains("completion_items="));
    assert!(!snapshot.contains("popup="));
    assert!(!snapshot.contains("query=/a candidates="));
}

#[test]
fn ratatui_default_snapshot_renders_empty_slash_completion_state() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![
            cell(
                WorkbenchCellKind::Model,
                "model",
                "provider=mock model=mock-chat",
            ),
            cell(
                WorkbenchCellKind::Session,
                "workspace",
                "path=/tmp/ikaros-workspace",
            ),
        ],
        timeline: vec![],
        main: vec![cell(
            WorkbenchCellKind::Error,
            "command completion",
            "query=/zzz candidates=0 action=tab_to_retry command=/commands /zzz",
        )],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "input_state: view=/zzz undo=0".into(),
    };
    let state = WorkbenchScreenState::default();

    let snapshot =
        render_tui_workbench_snapshot(&screen, &state, 96, 18).expect("ratatui snapshot");

    assert!(snapshot.contains("Slash Commands"));
    assert!(snapshot.contains("No matching slash commands."));
    assert!(!snapshot.contains("completion_items="));
    assert!(!snapshot.contains("query=/zzz candidates=0"));
}

#[test]
fn screen_navigation_focuses_side_panel_for_inline_approval_selection() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![cell(WorkbenchCellKind::Session, "turn", "ok")],
        main: vec![cell(WorkbenchCellKind::Context, "context", "ok")],
        side: vec![
            cell(WorkbenchCellKind::Approval, "first", "approve id-one"),
            cell(WorkbenchCellKind::Approval, "second", "approve id-two"),
        ],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let mut state = raw_state();

    state.apply(WorkbenchScreenAction::FocusPrevious);
    assert_eq!(state.focused_panel(), WorkbenchScreenPanel::Status);
    state.apply(WorkbenchScreenAction::FocusPrevious);
    state.apply(WorkbenchScreenAction::SelectNext);

    assert_eq!(state.focused_panel(), WorkbenchScreenPanel::Side);
    let frame = render_fullscreen_workbench_with_state(&screen, &state, 82, 15);

    assert!(frame.contains("Approvals / Queue*"));
    assert!(frame.contains("> [approval] second"));
    assert!(
        state
            .footer_summary()
            .contains("approval_action=/approval approve id")
    );
}

#[test]
fn screen_navigation_can_focus_status_panel() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![
            cell(WorkbenchCellKind::Model, "model", "command=/model"),
            cell(
                WorkbenchCellKind::Continuation,
                "queue",
                "command=/debug continuations",
            ),
        ],
        timeline: vec![cell(WorkbenchCellKind::Session, "turn", "ok")],
        main: vec![cell(WorkbenchCellKind::Context, "context", "ok")],
        side: vec![cell(WorkbenchCellKind::Approval, "approval", "pending")],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--raw", "--focus", "status", "--select", "2"])
        .expect("status selection");

    assert_eq!(state.focused_panel(), WorkbenchScreenPanel::Status);
    assert_eq!(
        screen_selected_cell_line(&screen, &state),
        "screen_selected: panel=status row=2 kind=continuation title=queue detail=command=/debug continuations"
    );
    assert_eq!(
        screen_selected_actions_line(&screen, &state),
        "screen_selected_actions: panel=status row=2 commands=/debug continuations"
    );
    let frame = render_fullscreen_workbench_with_state(&screen, &state, 100, 14);
    assert!(frame.contains("Status*"));
    assert!(state.footer_summary().contains("selected=status:2"));
}

#[test]
fn terminal_screen_keys_drive_navigation_and_selected_actions() {
    let mut state = raw_state();

    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
    ));
    assert_eq!(state.focused_panel(), WorkbenchScreenPanel::Main);
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)
    ));
    assert_eq!(state.focused_panel(), WorkbenchScreenPanel::Timeline);
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
    ));
    assert_eq!(state.scroll_for(WorkbenchScreenPanel::Timeline), 1);
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)
    ));
    assert_eq!(state.scroll_for(WorkbenchScreenPanel::Timeline), 0);
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)
    ));
    assert_eq!(state.selection_for(WorkbenchScreenPanel::Timeline), 1);
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)
    ));
    assert_eq!(state.selection_for(WorkbenchScreenPanel::Timeline), 0);
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    ));
    assert_eq!(
        state.take_open_action(),
        Some(WorkbenchScreenOpenAction::OpenSelected)
    );

    state.apply(WorkbenchScreenAction::FocusPrevious);
    assert_eq!(state.focused_panel(), WorkbenchScreenPanel::Status);
    state.apply(WorkbenchScreenAction::FocusPrevious);
    assert_eq!(state.focused_panel(), WorkbenchScreenPanel::Side);
    assert!(!apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)
    ));
    assert!(!apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)
    ));
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT)
    ));
    assert_eq!(
        state.take_approval_action(),
        Some(WorkbenchScreenApprovalAction::Approve)
    );
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT)
    ));
    assert_eq!(
        state.take_approval_action(),
        Some(WorkbenchScreenApprovalAction::Deny)
    );
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT)
    ));
    assert_eq!(
        state.take_continuation_action(),
        Some(WorkbenchScreenContinuationAction::Cancel)
    );
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)
    ));
    assert_eq!(
        state.take_input_action(),
        Some(WorkbenchScreenInputAction::Clear)
    );
    assert!(!apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)
    ));
}

#[test]
fn terminal_screen_keys_do_not_consume_bare_timeline_tab_letters() {
    let mut state = WorkbenchScreenState::default();
    assert_eq!(state.focused_panel(), WorkbenchScreenPanel::Timeline);

    for ch in ['c', 'p', 't', 'm', 'a', 'd', 'q', '!'] {
        assert!(
            !apply_workbench_screen_key_event(
                &mut state,
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
            ),
            "bare timeline shortcut {ch} should remain available for composer input"
        );
        assert_eq!(state.action_selection, None);
    }

    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)
    ));
    assert_eq!(state.action_selection.as_deref(), Some("timeline_all"));
}

#[test]
fn screen_navigation_selects_rows_in_the_focused_panel() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![
            cell(WorkbenchCellKind::Session, "timeline first", "a"),
            cell(WorkbenchCellKind::Session, "timeline second", "b"),
        ],
        main: vec![
            cell(WorkbenchCellKind::Context, "main first", "a"),
            cell(WorkbenchCellKind::Context, "main second", "b"),
        ],
        side: vec![
            cell(WorkbenchCellKind::Approval, "side first", "a"),
            cell(WorkbenchCellKind::Approval, "side second", "b"),
        ],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let mut state = parse_workbench_screen_state(&["--raw", "--focus", "main", "--select", "2"])
        .expect("main selection");

    let frame = render_fullscreen_workbench_with_state(&screen, &state, 120, 14);
    assert!(frame.contains("Main*"));
    assert!(frame.contains("> [context] main second"));
    assert!(frame.contains("selected panel=main row=2 kind=context"));
    assert!(frame.contains("title=main second action=/context"));
    assert!(!frame.contains("> [approval] side second"));
    assert!(frame.contains("selected panel=main row=2 kind=context"));

    state.apply(WorkbenchScreenAction::FocusPrevious);
    state.apply(WorkbenchScreenAction::SelectNext);
    let frame = render_fullscreen_workbench_with_state(&screen, &state, 120, 14);
    assert!(frame.contains("Timeline*"));
    assert!(frame.contains("> [session] timeline second"));
    assert!(frame.contains("selected panel=timeline row=2 kind=session"));
}

#[test]
fn screen_selected_cell_line_reports_the_focused_selection() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![cell(WorkbenchCellKind::Session, "timeline first", "a")],
        main: vec![
            cell(WorkbenchCellKind::Context, "main first", "a"),
            cell(WorkbenchCellKind::Context, "main second", "b"),
        ],
        side: vec![cell(WorkbenchCellKind::Approval, "side first", "a")],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--focus", "main", "--select", "2"])
        .expect("main selection");

    assert_eq!(
        screen_selected_cell_line(&screen, &state),
        "screen_selected: panel=main row=2 kind=context title=main second detail=b"
    );

    let missing = parse_workbench_screen_state(&["--focus", "side", "--select", "4"])
        .expect("missing selection");
    assert_eq!(
        screen_selected_cell_line(&screen, &missing),
        "screen_selected: panel=side row=4 kind=none title=none detail=none"
    );
}

#[test]
fn screen_selected_actions_line_reports_navigation_commands() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![cell(
            WorkbenchCellKind::Model,
            "event model_stream turn=turn-one",
            "correlation=session:session-one:turn:turn-one event=event-one",
        )],
        main: vec![cell(
            WorkbenchCellKind::Context,
            "context budget",
            "command=/context",
        )],
        side: vec![
            cell(
                WorkbenchCellKind::Approval,
                "pending approval-one",
                "approve=/approval approve approval-one deny=/approval deny approval-one",
            ),
            cell(
                WorkbenchCellKind::Continuation,
                "queue next_turn",
                "id=cont-one status=queued cancel=/cancel cont-one",
            ),
        ],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };

    let timeline = parse_workbench_screen_state(&["--focus", "timeline", "--select", "1"])
        .expect("timeline selection");
    assert_eq!(
        screen_selected_actions_line(&screen, &timeline),
        "screen_selected_actions: panel=timeline row=1 commands=/timeline turn-one | /trace turn-one | /debug turn-one"
    );

    let main = parse_workbench_screen_state(&["--focus", "main", "--select", "1"])
        .expect("main selection");
    assert_eq!(
        screen_selected_actions_line(&screen, &main),
        "screen_selected_actions: panel=main row=1 commands=/context"
    );

    let side = parse_workbench_screen_state(&["--focus", "side", "--select", "2"])
        .expect("side selection");
    assert_eq!(
        screen_selected_actions_line(&screen, &side),
        "screen_selected_actions: panel=side row=2 commands=/cancel cont-one"
    );
}

#[test]
fn screen_selected_actions_can_select_main_cell_by_title() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![
            cell(
                WorkbenchCellKind::Context,
                "context budget",
                "command=/context",
            ),
            cell(
                WorkbenchCellKind::Model,
                "provider cost",
                "command=/provider matrix",
            ),
            cell(
                WorkbenchCellKind::Memory,
                "memory",
                "command=/debug memory-lifecycle session-one memory=/memory",
            ),
        ],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--focus", "main", "--select-title", "memory"])
        .expect("title selection");

    assert_eq!(
        screen_selected_cell_line(&screen, &state),
        "screen_selected: panel=main row=3 kind=memory title=memory detail=command=/debug memory-lifecycle session-one memory=/memory"
    );
    assert_eq!(
        screen_selected_actions_line(&screen, &state),
        "screen_selected_actions: panel=main row=3 commands=/debug memory-lifecycle session-one | /memory"
    );
    assert_eq!(
        screen_selected_primary_action(&screen, &state).as_deref(),
        Some("/debug memory-lifecycle session-one")
    );
}

#[test]
fn screen_selected_actions_can_select_main_cell_by_kind() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![
            cell(
                WorkbenchCellKind::Context,
                "context budget",
                "command=/context",
            ),
            cell(
                WorkbenchCellKind::Memory,
                "memory projection",
                "command=/debug memory-lifecycle session-one memory=/memory",
            ),
        ],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--focus", "main", "--select-kind", "memory"])
        .expect("kind selection");

    assert_eq!(
        screen_selected_cell_line(&screen, &state),
        "screen_selected: panel=main row=2 kind=memory title=memory projection detail=command=/debug memory-lifecycle session-one memory=/memory"
    );
    assert_eq!(
        screen_selected_primary_action(&screen, &state).as_deref(),
        Some("/debug memory-lifecycle session-one")
    );
}

#[test]
fn screen_selected_actions_can_select_main_cell_by_multi_word_title() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![
            cell(
                WorkbenchCellKind::Context,
                "context budget",
                "command=/context",
            ),
            cell(
                WorkbenchCellKind::Model,
                "provider cost",
                "command=/provider matrix",
            ),
        ],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let mut state = parse_workbench_screen_state(&[
        "--focus",
        "main",
        "--select-title",
        "provider",
        "cost",
        "open-selected",
    ])
    .expect("multi-word title selection");

    assert_eq!(
        screen_selected_cell_line(&screen, &state),
        "screen_selected: panel=main row=2 kind=model title=provider cost detail=command=/provider matrix"
    );
    assert_eq!(
        screen_selected_primary_action(&screen, &state).as_deref(),
        Some("/provider matrix")
    );
    assert_eq!(
        state.take_open_action(),
        Some(WorkbenchScreenOpenAction::OpenSelected)
    );
}

#[test]
fn screen_selected_actions_can_select_main_cell_by_action() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![
            cell(
                WorkbenchCellKind::Model,
                "provider matrix",
                "command=/provider matrix debug=/provider debug",
            ),
            cell(
                WorkbenchCellKind::Model,
                "provider cost",
                "command=/provider debug matrix=/provider matrix debug=/provider debug",
            ),
        ],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let mut state = parse_workbench_screen_state(&[
        "--focus",
        "main",
        "--select-action",
        "/provider",
        "debug",
        "open-selected",
    ])
    .expect("action selection");

    assert_eq!(
        screen_selected_cell_line(&screen, &state),
        "screen_selected: panel=main row=2 kind=model title=provider cost detail=command=/provider debug matrix=/provider matrix debug=/provider debug"
    );
    assert_eq!(
        screen_selected_primary_action(&screen, &state).as_deref(),
        Some("/provider debug")
    );
    assert_eq!(
        state.take_open_action(),
        Some(WorkbenchScreenOpenAction::OpenSelected)
    );
}

#[test]
fn screen_selected_actions_line_deduplicates_repeated_commands() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![cell(
            WorkbenchCellKind::Model,
            "provider matrix",
            "command=/provider matrix matrix=/provider matrix live=/provider matrix --live",
        )],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--focus", "main"]).expect("main selection");

    assert_eq!(
        screen_selected_actions_line(&screen, &state),
        "screen_selected_actions: panel=main row=1 commands=/provider matrix | /provider matrix --live"
    );
}

#[test]
fn screen_selected_actions_line_reports_coding_workflow_commands() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![cell(
            WorkbenchCellKind::Coding,
            "coding test",
            "command=/diff test=/code test review=/code review rollback=/code rollback",
        )],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--focus", "main"]).expect("main selection");

    assert_eq!(
        screen_selected_actions_line(&screen, &state),
        "screen_selected_actions: panel=main row=1 commands=/diff | /code test | /code review | /code rollback"
    );
}

#[test]
fn screen_selected_actions_line_reports_context_and_tool_summary_commands() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![cell(
            WorkbenchCellKind::Context,
            "context progress summary",
            "trace=/trace --kind context context=/context tools=/tools",
        )],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--focus", "main"]).expect("main selection");

    assert_eq!(
        screen_selected_actions_line(&screen, &state),
        "screen_selected_actions: panel=main row=1 commands=/trace --kind context | /context | /tools"
    );
}

#[test]
fn screen_selected_actions_json_line_reports_structured_commands() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![cell(
            WorkbenchCellKind::Model,
            "provider matrix",
            "command=/provider matrix matrix=/provider matrix live=/provider matrix --live",
        )],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--focus", "main"]).expect("main selection");

    let line = screen_selected_actions_json_line(&screen, &state);
    let payload = line
        .strip_prefix("screen_selected_actions_json: ")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .expect("structured action payload");

    assert_eq!(payload["panel"], "main");
    assert_eq!(payload["row"], 1);
    assert_eq!(payload["kind"], "model");
    assert_eq!(
        payload["commands"],
        serde_json::json!(["/provider matrix", "/provider matrix --live"])
    );
}

#[test]
fn screen_json_line_exports_visible_screen_snapshot_without_secret_leakage() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![cell(WorkbenchCellKind::Model, "model", "token=sk-secret")],
        timeline: vec![cell(
            WorkbenchCellKind::Session,
            "turn",
            "turn=turn-one detail=ok",
        )],
        main: vec![cell(
            WorkbenchCellKind::Context,
            "context budget",
            "command=/context token=sk-secret",
        )],
        side: vec![cell(
            WorkbenchCellKind::Approval,
            "approval",
            "approve=/approval approve id-one",
        )],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--focus", "main", "--select", "1"])
        .expect("main selection");

    let line = screen_json_line(&screen, &state);
    let payload = line
        .strip_prefix("screen_json: ")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .expect("screen JSON payload");

    assert_eq!(payload["title"], "Ikaros Workbench");
    assert_eq!(payload["state"]["focused_panel"], "main");
    assert_eq!(payload["state"]["selection"]["main"], 1);
    assert_eq!(payload["panels"]["main"][0]["kind"], "context");
    assert_eq!(payload["panels"]["main"][0]["title"], "context budget");
    assert_eq!(payload["selected"]["kind"], "context");
    assert_eq!(
        payload["selected"]["commands"],
        serde_json::json!(["/context"])
    );
    assert_eq!(payload["modal"], serde_json::Value::Null);
    let serialized = serde_json::to_string(&payload).expect("serialize payload");
    assert!(!serialized.contains("sk-secret"));
    assert!(serialized.contains("[REDACTED_SECRET]"));
}

#[test]
fn screen_json_line_exports_protocol_and_key_binding_metadata() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![cell(WorkbenchCellKind::Session, "turn", "turn=turn-one")],
        main: vec![cell(
            WorkbenchCellKind::Context,
            "context",
            "command=/context",
        )],
        side: vec![cell(
            WorkbenchCellKind::Approval,
            "approval",
            "approve=/approval approve id-one deny=/approval deny id-one",
        )],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--focus", "side", "--select", "1"])
        .expect("side selection");

    let line = screen_json_line(&screen, &state);
    let payload = line
        .strip_prefix("screen_json: ")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .expect("screen JSON payload");
    let bindings = payload["key_bindings"]
        .as_array()
        .expect("key bindings array");

    assert_eq!(payload["schema"], "ikaros-workbench-screen-v1");
    assert_eq!(payload["version"], 1);
    assert_eq!(payload["modal"]["kind"], "approval");
    assert_eq!(
        payload["modal"]["actions"]["approve_selected"],
        "/screen approve-selected"
    );
    assert_eq!(
        payload["modal"]["actions"]["deny_selected"],
        "/screen deny-selected"
    );
    assert_eq!(payload["overlay_routing"]["active_overlay"], "approval");
    assert_eq!(payload["overlay_routing"]["modal_scope"], "approval");
    assert_eq!(
        payload["overlay_routing"]["enter_target"],
        "/screen --focus side --select-title pending id-one"
    );
    assert_eq!(
        payload["surface"]["action_menu_model"]["priority"]["default_group"],
        "approval"
    );
    assert_eq!(
        payload["surface"]["action_menu_model"]["primary"]["command"],
        "/screen --focus side --select-title pending id-one"
    );
    assert_eq!(
        payload["modal"]["primary"]["command"],
        "/screen --focus side --select-title pending id-one"
    );
    assert!(bindings.iter().any(|binding| {
        binding["key"] == "tab"
            && binding["action"] == "focus_next"
            && binding["command"] == "/screen --focus-next"
    }));
    assert!(bindings.iter().any(|binding| {
        binding["key"] == "enter"
            && binding["action"] == "open_selected"
            && binding["command"] == "/screen open-selected"
    }));
    assert!(bindings.iter().any(|binding| {
        binding["key"] == "alt+a"
            && binding["action"] == "approve_selected"
            && binding["command"] == "/screen approve-selected"
    }));
}

#[test]
fn screen_queue_panel_exports_primary_selection_state() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![cell(
            WorkbenchCellKind::Session,
            "bottom pane",
            "active_view=input_queue approvals=0 pending_inputs=1 next_input=queued attachments=0 continuations=1",
        )],
        timeline: vec![],
        main: vec![],
        side: vec![
            cell(
                WorkbenchCellKind::Continuation,
                "queue",
                "queued=0 running=1 completed=0 failed=0 cancelled=0",
            ),
            cell(
                WorkbenchCellKind::Continuation,
                "queue next_turn",
                "id=cont-one status=running reason=resume turn=turn-one cancel=/cancel cont-one",
            ),
            cell(
                WorkbenchCellKind::Continuation,
                "queue controls",
                "run=/queue run cancel=/cancel all inspect=/debug continuations",
            ),
        ],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };

    let queue = screen_queue_panel_json(&screen);

    assert_eq!(queue["primary_command"], "/cancel cont-one");
    assert_eq!(queue["primary"]["command"], "/cancel cont-one");
    assert_eq!(queue["selection_state"]["has_active_item"], true);
    assert_eq!(queue["selection_state"]["can_cancel"], true);
    assert_eq!(queue["selection_state"]["can_run"], true);

    let items = action_menu_queue_items_json(&screen);
    assert_eq!(
        items.first().and_then(|item| item["command"].as_str()),
        Some("/cancel cont-one")
    );

    let line = screen_json_line(&screen, &WorkbenchScreenState::default());
    let payload = line
        .strip_prefix("screen_json: ")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .expect("screen JSON payload");
    assert_eq!(
        payload["surface"]["action_menu_model"]["priority"]["default_group"],
        "queue"
    );
    assert_eq!(
        payload["surface"]["action_menu_model"]["primary"]["command"],
        "/cancel cont-one"
    );
    assert_eq!(
        payload["surface"]["turn_state_model"]["interrupt"]["available"],
        true
    );
    assert_eq!(
        payload["surface"]["turn_state_model"]["primary"]["command"],
        "/cancel all"
    );
}

#[test]
fn screen_selected_primary_action_returns_first_safe_navigation_command() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![cell(
            WorkbenchCellKind::Model,
            "event model_stream turn=turn-one",
            "correlation=session:session-one:turn:turn-one event=event-one",
        )],
        main: vec![cell(
            WorkbenchCellKind::Context,
            "context budget",
            "command=/context",
        )],
        side: vec![cell(
            WorkbenchCellKind::Continuation,
            "queue next_turn",
            "id=cont-one status=queued cancel=/cancel cont-one",
        )],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };

    let timeline = parse_workbench_screen_state(&["--focus", "timeline"]).expect("timeline");
    assert_eq!(
        screen_selected_primary_action(&screen, &timeline).as_deref(),
        Some("/timeline turn-one")
    );

    let main = parse_workbench_screen_state(&["--focus", "main"]).expect("main");
    assert_eq!(
        screen_selected_primary_action(&screen, &main).as_deref(),
        Some("/context")
    );

    let side = parse_workbench_screen_state(&["--focus", "side"]).expect("side");
    assert_eq!(
        screen_selected_primary_action(&screen, &side).as_deref(),
        Some("/cancel cont-one")
    );

    let error =
        parse_workbench_screen_state(&["--focus", "timeline", "--select-action", "/status"])
            .expect("status action");
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![cell(
            WorkbenchCellKind::Error,
            "event error turn=turn-one",
            "command=/status trace=/trace --failed",
        )],
        main: vec![],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    assert_eq!(
        screen_selected_primary_action(&screen, &error).as_deref(),
        Some("/status")
    );
}

#[test]
fn fullscreen_frame_exposes_selected_primary_action_in_footer() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![],
        side: vec![cell(
            WorkbenchCellKind::Continuation,
            "queue next_turn",
            "id=cont-one status=queued cancel=/cancel cont-one",
        )],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--raw", "--focus", "side"]).expect("side");

    let frame = render_fullscreen_workbench_with_state(&screen, &state, 96, 12);

    assert!(frame.contains("enter=/cancel cont-one"));
}

#[test]
fn screen_navigation_parses_workbench_screen_arguments() {
    let state = parse_workbench_screen_state(&[
        "--raw", "--focus", "main", "--scroll", "2", "--select", "3", "--down",
    ])
    .expect("screen state");

    assert_eq!(state.focused_panel(), WorkbenchScreenPanel::Main);
    let frame = render_fullscreen_workbench_with_state(
        &WorkbenchScreen {
            title: "Ikaros Workbench".into(),
            status: vec![],
            timeline: vec![cell(WorkbenchCellKind::Session, "timeline", "ok")],
            main: vec![
                cell(WorkbenchCellKind::Context, "hidden one", "a"),
                cell(WorkbenchCellKind::Context, "hidden two", "b"),
                cell(WorkbenchCellKind::Context, "visible", "c"),
                cell(WorkbenchCellKind::Context, "also visible", "d"),
            ],
            side: vec![cell(WorkbenchCellKind::Approval, "approval", "pending")],
            footer: "session=session-one".into(),
            input_hint: "type".into(),
        },
        &state,
        86,
        14,
    );

    assert!(frame.contains("Main*"));
    assert!(frame.contains("visible"));
    assert!(!frame.contains("hidden one"));
    assert!(frame.contains("scroll=main:3"));
}

#[test]
fn screen_navigation_parses_fullscreen_mode_toggle() {
    let fullscreen = parse_workbench_screen_state(&["--fullscreen"]).expect("fullscreen");
    assert!(fullscreen.fullscreen());

    let inline = parse_workbench_screen_state(&["--fullscreen", "--inline"]).expect("inline");
    assert!(!inline.fullscreen());
}

#[test]
fn screen_navigation_parses_selected_approval_actions() {
    let mut approve = parse_workbench_screen_state(&["approve-selected"]).expect("approve");
    assert_eq!(
        approve.take_approval_action(),
        Some(WorkbenchScreenApprovalAction::Approve)
    );
    assert_eq!(approve.take_approval_action(), None);

    let mut deny = parse_workbench_screen_state(&["deny-selected"]).expect("deny");
    assert_eq!(
        deny.take_approval_action(),
        Some(WorkbenchScreenApprovalAction::Deny)
    );
}

#[test]
fn screen_navigation_parses_selected_continuation_cancel_action() {
    let mut cancel =
        parse_workbench_screen_state(&["cancel-selected"]).expect("cancel continuation");

    assert_eq!(
        cancel.take_continuation_action(),
        Some(WorkbenchScreenContinuationAction::Cancel)
    );
    assert_eq!(cancel.take_continuation_action(), None);
}

#[test]
fn screen_navigation_parses_selected_input_clear_action() {
    let mut clear = parse_workbench_screen_state(&["clear-selected"]).expect("clear input");

    assert_eq!(
        clear.take_input_action(),
        Some(WorkbenchScreenInputAction::Clear)
    );
    assert_eq!(clear.take_input_action(), None);
}

#[test]
fn screen_navigation_parses_selected_open_action() {
    let mut open = parse_workbench_screen_state(&["open-selected"]).expect("open selected");

    assert_eq!(
        open.take_open_action(),
        Some(WorkbenchScreenOpenAction::OpenSelected)
    );
    assert_eq!(open.take_open_action(), None);
}

#[test]
fn screen_navigation_parses_key_aliases_for_selected_actions() {
    let mut open = parse_workbench_screen_state(&["enter"]).expect("enter");
    assert_eq!(
        open.take_open_action(),
        Some(WorkbenchScreenOpenAction::OpenSelected)
    );

    let mut approve = parse_workbench_screen_state(&["a"]).expect("approve alias");
    assert_eq!(
        approve.take_approval_action(),
        Some(WorkbenchScreenApprovalAction::Approve)
    );

    let mut deny = parse_workbench_screen_state(&["d"]).expect("deny alias");
    assert_eq!(
        deny.take_approval_action(),
        Some(WorkbenchScreenApprovalAction::Deny)
    );

    let mut cancel = parse_workbench_screen_state(&["c"]).expect("cancel alias");
    assert_eq!(
        cancel.take_continuation_action(),
        Some(WorkbenchScreenContinuationAction::Cancel)
    );
}

#[test]
fn screen_command_palette_parses_query_and_open_selected_action() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let mut state = parse_workbench_screen_state(&["--palette-query", "/help", "open-selected"])
        .expect("palette query");

    assert!(state.command_palette_open());
    assert_eq!(state.command_palette_query.as_deref(), Some("/help"));
    assert_eq!(
        screen_selected_primary_action(&screen, &state).as_deref(),
        Some("/help")
    );
    assert_eq!(
        state.take_open_action(),
        Some(WorkbenchScreenOpenAction::OpenSelected)
    );

    let frame = render_fullscreen_workbench_with_state(&screen, &state, 100, 14);
    assert!(frame.contains("/help"));

    let raw_mode =
        parse_workbench_screen_state(&["--palette", "raw"]).expect("palette plus raw render mode");
    assert!(raw_mode.command_palette_open());
    assert_eq!(raw_mode.command_palette_query.as_deref(), None);
    assert!(raw_mode.raw_mode());

    let help_query =
        parse_workbench_screen_state(&["--palette", "help"]).expect("palette help query");
    assert!(help_query.command_palette_open());
    assert_eq!(help_query.command_palette_query.as_deref(), Some("help"));

    let raw_query = parse_workbench_screen_state(&["--palette-query", "raw"])
        .expect("explicit raw palette query");
    assert!(raw_query.command_palette_open());
    assert_eq!(raw_query.command_palette_query.as_deref(), Some("raw"));
}

#[test]
fn screen_command_palette_key_events_capture_picker_navigation() {
    let mut state = WorkbenchScreenState::default();

    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE)
    ));
    assert!(state.command_palette_open());
    assert_eq!(state.command_palette_selection, 0);

    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
    ));
    assert_eq!(state.action_selection.as_deref(), Some("global_palette"));
    assert!(state.command_palette_selection > 0);

    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)
    ));
    assert_eq!(state.command_palette_selection, 0);

    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    ));
    assert_eq!(
        state.take_open_action(),
        Some(WorkbenchScreenOpenAction::OpenSelected)
    );
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)
    ));
    assert_eq!(
        state.take_open_action(),
        Some(WorkbenchScreenOpenAction::ConfirmSelected)
    );
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT)
    ));
    assert_eq!(state.take_approval_action(), None);

    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    ));
    assert!(!state.command_palette_open());
}

#[test]
fn screen_command_palette_key_events_edit_query() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let mut state = WorkbenchScreenState::default();

    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE)
    ));
    for ch in ['/', 'h', 'e', 'l', 'p'] {
        assert!(apply_workbench_screen_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
        ));
    }
    assert_eq!(state.command_palette_query.as_deref(), Some("/help"));
    assert_eq!(
        screen_selected_primary_action(&screen, &state).as_deref(),
        Some("/help")
    );

    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
    ));
    assert_eq!(state.command_palette_query.as_deref(), Some("/hel"));

    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)
    ));
    assert_eq!(state.command_palette_query, None);
    assert_eq!(state.command_palette_selection, 0);

    for ch in ['c', 'o', 'd', 'e', ' ', 'r'] {
        assert!(apply_workbench_screen_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
        ));
    }
    assert_eq!(state.command_palette_query.as_deref(), Some("code r"));
}

#[test]
fn screen_command_palette_enter_is_noop_without_matches() {
    let mut state = parse_workbench_screen_state(&["--palette-query", "zzzzzz-no-command"])
        .expect("empty palette query");

    assert!(state.command_palette_open());
    assert_eq!(state.selected_command_palette_command(), None);
    let popup = command_palette_overlay_json(&state);
    assert_eq!(popup["empty"], true);
    assert_eq!(popup["accept_enabled"], false);
    assert_eq!(popup["enter_noop_when_empty"], true);
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    ));
    assert_eq!(state.take_open_action(), None);
    assert!(apply_workbench_screen_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)
    ));
    assert_eq!(state.take_open_action(), None);
}

#[test]
fn screen_command_palette_close_clears_palette_action_selection() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![cell(
            WorkbenchCellKind::Context,
            "context",
            "command=/context",
        )],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let mut state = parse_workbench_screen_state(&["--focus", "main", "--palette-query", "/help"])
        .expect("palette query");

    assert!(state.command_palette_open());
    assert_eq!(state.action_selection.as_deref(), Some("global_palette"));
    assert_eq!(
        screen_selected_primary_action(&screen, &state).as_deref(),
        Some("/help")
    );

    assert!(state.close_command_palette());

    assert!(!state.command_palette_open());
    assert_eq!(state.action_selection, None);
    assert_eq!(
        screen_selected_primary_action(&screen, &state).as_deref(),
        Some("/context")
    );
}

#[test]
fn approval_overlay_takes_primary_precedence_over_open_palette() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![],
        side: vec![cell(
            WorkbenchCellKind::Approval,
            "pending id-one",
            "approve=/approval approve id-one deny=/approval deny id-one",
        )],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--palette-query", "/help"]).expect("palette query");

    assert_eq!(
        screen_selected_primary_action(&screen, &state).as_deref(),
        Some("/screen --focus side --select-title pending id-one")
    );

    let line = screen_json_line(&screen, &state);
    let payload = line
        .strip_prefix("screen_json: ")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .expect("screen JSON payload");
    assert_eq!(payload["modal_kind"], "approval");
    assert_eq!(payload["overlay_routing"]["active_overlay"], "approval");
    assert_eq!(
        payload["surface"]["action_menu_model"]["primary"]["command"],
        "/screen --focus side --select-title pending id-one"
    );
    assert_eq!(
        payload["surface"]["turn_state_model"]["state"],
        "approval_pending"
    );
}

#[test]
fn screen_json_line_exports_command_palette_modal_state() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![],
        timeline: vec![],
        main: vec![],
        side: vec![],
        footer: "session=session-one".into(),
        input_hint: "type".into(),
    };
    let state = parse_workbench_screen_state(&["--palette-query", "/help"]).expect("palette query");

    let line = screen_json_line(&screen, &state);
    let payload = line
        .strip_prefix("screen_json: ")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .expect("screen JSON payload");

    assert_eq!(payload["modal_kind"], "command_palette");
    assert_eq!(payload["surface"]["modal"], "command_palette");
    let popup = &payload["surface"]["status_surfaces"]["command_palette"]["popup"];
    assert_eq!(popup["kind"], "command_palette");
    assert_eq!(popup["selected_command"], "/help");
    assert_eq!(
        payload["surface"]["action_menu_model"]["primary"]["command"],
        "/help"
    );
    assert_eq!(popup["selected_item"]["command"], "/help");
    assert_eq!(popup["match_count"], 1);
    assert_eq!(popup["visible_count"], 1);
    assert_eq!(payload["keymap_model"]["active_scope"], "command_palette");
    assert_eq!(payload["keymap_model"]["modal_scope"], "command_palette");
    assert_eq!(
        payload["overlay_routing"]["active_overlay"],
        "command_palette"
    );
    assert_eq!(
        payload["overlay_routing"]["text_target"],
        "command_palette_filter"
    );
    assert_eq!(payload["overlay_routing"]["enter_target"], "/help");
    assert_eq!(
        payload["surface"]["bottom_pane_model"]["routing"]["enter"],
        "/help"
    );
    assert_eq!(
        payload["surface"]["input_model"]["text_target"],
        "command_palette_filter"
    );
    assert_eq!(
        payload["surface"]["turn_state_model"]["state"],
        "input_blocked"
    );
    assert_eq!(payload["surface"]["turn_state_model"]["can_submit"], false);
}

#[test]
fn screen_navigation_rejects_unknown_screen_arguments() {
    let error = parse_workbench_screen_state(&["--focus", "missing"]).expect_err("error");

    assert!(error.to_string().contains("unknown screen focus panel"));
}

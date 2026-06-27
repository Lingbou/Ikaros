use super::{
    WorkbenchInputAction, WorkbenchInputEvent, WorkbenchInputState, WorkbenchTerminalInputEvent,
    WorkbenchTerminalInputOutcome, apply_workbench_terminal_input_event,
    format_workbench_input_state, parse_workbench_input_event, parse_workbench_terminal_key_event,
};
use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};

#[test]
fn terminal_key_events_map_to_workbench_input_events() {
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::CompletionPrevious
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::CompletionNext
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::CompletionPagePrevious
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::CompletionPageNext
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveLeft
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveRight
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveStart
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveEnd
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::DeletePrevious
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::DeleteNext
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::Complete
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL
        )),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::HistoryPrevious
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL
        )),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::HistoryNext
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL
        )),
        Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::Undo
        ))
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Submit)
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
        Some(WorkbenchTerminalInputEvent::InsertNewline)
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
        Some(WorkbenchTerminalInputEvent::InsertNewline)
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::CONTROL
        )),
        Some(WorkbenchTerminalInputEvent::InsertNewline)
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::Escape)
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )),
        Some(WorkbenchTerminalInputEvent::Interrupt)
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL
        )),
        Some(WorkbenchTerminalInputEvent::EndOfInput)
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::CONTROL
        )),
        Some(WorkbenchTerminalInputEvent::ClearSession)
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        Some(WorkbenchTerminalInputEvent::InsertText("x".into()))
    );
}

#[test]
fn terminal_key_events_ignore_system_shortcuts() {
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::SHIFT | KeyModifiers::SUPER
        )),
        None
    );
    assert_eq!(
        parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::META)),
        None
    );
}

#[test]
fn workbench_input_state_recalls_history_and_completes_slash_commands() {
    let mut input = WorkbenchInputState::default();
    input.record_history("first message");
    input.record_history("/status");

    assert_eq!(
        input.apply(WorkbenchInputAction::HistoryPrevious),
        Some("/status".into())
    );
    assert_eq!(
        input.apply(WorkbenchInputAction::HistoryPrevious),
        Some("first message".into())
    );
    assert_eq!(
        input.apply(WorkbenchInputAction::HistoryNext),
        Some("/status".into())
    );

    input.set_buffer("/sta");
    assert_eq!(input.completion_candidates(), vec!["/status"]);
    assert_eq!(
        input.apply(WorkbenchInputAction::Complete),
        Some("/status ".into())
    );
    assert_eq!(input.buffer(), "/status ");

    input.record_history("token=sk-secret-value");
    assert!(
        !input
            .history_entries()
            .iter()
            .any(|entry| entry.contains("sk-secret-value"))
    );
}

#[test]
fn workbench_input_state_edits_at_cursor_and_undoes_last_change() {
    let mut input = WorkbenchInputState::default();

    input.insert_text("helo");
    assert_eq!(input.buffer(), "helo");
    assert_eq!(input.cursor(), 4);

    input.apply(WorkbenchInputAction::MoveLeft);
    input.insert_text("l");
    assert_eq!(input.buffer(), "hello");
    assert_eq!(input.cursor(), 4);

    input.apply(WorkbenchInputAction::DeletePrevious);
    assert_eq!(input.buffer(), "helo");
    assert_eq!(input.cursor(), 3);

    input.apply(WorkbenchInputAction::Undo);
    assert_eq!(input.buffer(), "hello");
    assert_eq!(input.cursor(), 4);

    input.apply(WorkbenchInputAction::MoveRight);
    input.apply(WorkbenchInputAction::DeleteNext);
    assert_eq!(input.buffer(), "hello");
    assert_eq!(input.cursor(), 5);
}

#[test]
fn workbench_input_state_completion_keeps_cursor_at_end() {
    let mut input = WorkbenchInputState::default();

    input.insert_text("/sta");
    assert_eq!(
        input.apply(WorkbenchInputAction::Complete),
        Some("/status ".into())
    );

    assert_eq!(input.buffer(), "/status ");
    assert_eq!(input.cursor(), "/status ".chars().count());
    input.apply(WorkbenchInputAction::Undo);
    assert_eq!(input.buffer(), "/sta");
    assert_eq!(input.cursor(), 4);
}

#[test]
fn workbench_input_state_renders_cursor_view_and_completion_candidates() {
    let mut input = WorkbenchInputState::default();
    input.insert_text("/sta");
    input.apply(WorkbenchInputAction::MoveLeft);

    let line = format_workbench_input_state("move_left", &input);

    assert!(line.contains("input_state: action=move_left"));
    assert!(line.contains("cursor=3"));
    assert!(line.contains("buffer=/sta"));
    assert!(line.contains("view=/st|a"));
    assert!(line.contains("completion_candidates=/status"));

    input.set_buffer("token=sk-secret-value");
    let redacted = format_workbench_input_state("set_buffer", &input);
    assert!(redacted.contains("[REDACTED_SECRET]"));
    assert!(!redacted.contains("sk-secret-value"));
}

#[test]
fn workbench_input_state_moves_to_line_start_and_end() {
    let mut input = WorkbenchInputState::default();

    input.insert_text("abc");
    input.apply(WorkbenchInputAction::MoveStart);
    input.insert_text(">");
    assert_eq!(input.buffer(), ">abc");
    assert_eq!(input.cursor(), 1);

    input.apply(WorkbenchInputAction::MoveEnd);
    input.insert_text("<");
    assert_eq!(input.buffer(), ">abc<");
    assert_eq!(input.cursor(), 5);
}

#[test]
fn workbench_input_state_kills_text_around_cursor() {
    let mut input = WorkbenchInputState::default();

    input.insert_text("hello world");
    for _ in 0..5 {
        input.apply(WorkbenchInputAction::MoveLeft);
    }
    input.apply(WorkbenchInputAction::DeleteBeforeCursor);
    assert_eq!(input.buffer(), "world");
    assert_eq!(input.cursor(), 0);

    input.apply(WorkbenchInputAction::Undo);
    assert_eq!(input.buffer(), "hello world");
    assert_eq!(input.cursor(), 6);

    input.apply(WorkbenchInputAction::MoveLeft);
    input.apply(WorkbenchInputAction::DeleteAfterCursor);
    assert_eq!(input.buffer(), "hello");
    assert_eq!(input.cursor(), 5);
}

#[test]
fn workbench_input_event_adapter_maps_terminal_control_sequences() {
    assert_eq!(
        parse_workbench_input_event("\u{1b}[A"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::CompletionPrevious)
    );
    assert_eq!(
        parse_workbench_input_event("\u{1b}[B"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::CompletionNext)
    );
    assert_eq!(
        parse_workbench_input_event("\u{1b}[D"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveLeft)
    );
    assert_eq!(
        parse_workbench_input_event("\u{1b}[C"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveRight)
    );
    assert_eq!(
        parse_workbench_input_event("\u{1b}[H"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveStart)
    );
    assert_eq!(
        parse_workbench_input_event("\u{1b}[F"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveEnd)
    );
    assert_eq!(
        parse_workbench_input_event("\u{7f}"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeletePrevious)
    );
    assert_eq!(
        parse_workbench_input_event("\u{1b}[3~"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteNext)
    );
    assert_eq!(
        parse_workbench_input_event("\u{10}"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistoryPrevious)
    );
    assert_eq!(
        parse_workbench_input_event("\u{e}"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistoryNext)
    );
    assert_eq!(
        parse_workbench_input_event("\u{2}"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveLeft)
    );
    assert_eq!(
        parse_workbench_input_event("\u{6}"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveRight)
    );
    assert_eq!(
        parse_workbench_input_event("\u{4}"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteNext)
    );
    assert_eq!(
        parse_workbench_input_event("\u{15}"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteBeforeCursor)
    );
    assert_eq!(
        parse_workbench_input_event("\u{b}"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteAfterCursor)
    );
    assert_eq!(
        parse_workbench_input_event("\u{1a}"),
        WorkbenchInputEvent::Action(WorkbenchInputAction::Undo)
    );
}

#[test]
fn workbench_input_event_adapter_extracts_completion_prefix_without_redaction_leakage() {
    assert_eq!(
        parse_workbench_input_event("/sta\t"),
        WorkbenchInputEvent::CompletePrefix("/sta".into())
    );
    assert_eq!(
        parse_workbench_input_event("hello"),
        WorkbenchInputEvent::SubmitLine("hello".into())
    );
    assert_eq!(
        parse_workbench_input_event("token=sk-secret\t"),
        WorkbenchInputEvent::CompletePrefix("[REDACTED_SECRET]".into())
    );
}

#[test]
fn terminal_input_reducer_edits_submits_clears_and_exits_without_line_mode() {
    let mut state = WorkbenchInputState::from_history(["/status".into()]);

    assert_eq!(
        apply_workbench_terminal_input_event(
            &mut state,
            WorkbenchTerminalInputEvent::InsertText("/sta".into())
        ),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(state.buffer(), "/sta");
    assert_eq!(
        apply_workbench_terminal_input_event(
            &mut state,
            WorkbenchTerminalInputEvent::Action(WorkbenchInputAction::Complete)
        ),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(state.buffer(), "/status ");
    assert_eq!(
        apply_workbench_terminal_input_event(&mut state, WorkbenchTerminalInputEvent::Submit),
        WorkbenchTerminalInputOutcome::Submit("/status".into())
    );
    assert_eq!(state.buffer(), "");
    assert_eq!(
        apply_workbench_terminal_input_event(&mut state, WorkbenchTerminalInputEvent::Submit),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(
        apply_workbench_terminal_input_event(&mut state, WorkbenchTerminalInputEvent::Interrupt),
        WorkbenchTerminalInputOutcome::Exit
    );

    state.set_buffer("draft");
    assert_eq!(
        apply_workbench_terminal_input_event(&mut state, WorkbenchTerminalInputEvent::Interrupt),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(state.buffer(), "");
}

#[test]
fn terminal_input_reducer_preserves_explicit_multiline_messages() {
    let mut state = WorkbenchInputState::default();

    assert_eq!(
        apply_workbench_terminal_input_event(
            &mut state,
            WorkbenchTerminalInputEvent::InsertText("first".into())
        ),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(
        apply_workbench_terminal_input_event(
            &mut state,
            WorkbenchTerminalInputEvent::InsertNewline
        ),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(
        apply_workbench_terminal_input_event(
            &mut state,
            WorkbenchTerminalInputEvent::InsertText("second".into())
        ),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(state.buffer(), "first\nsecond");
    assert_eq!(
        apply_workbench_terminal_input_event(&mut state, WorkbenchTerminalInputEvent::Submit),
        WorkbenchTerminalInputOutcome::Submit("first\nsecond".into())
    );
}

#[test]
fn terminal_input_reducer_drops_bare_mouse_fragments_from_buffer() {
    let mut state = WorkbenchInputState::default();

    assert_eq!(
        apply_workbench_terminal_input_event(
            &mut state,
            WorkbenchTerminalInputEvent::InsertText("[<35;55;37M".into())
        ),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(state.buffer(), "");

    assert_eq!(
        apply_workbench_terminal_input_event(
            &mut state,
            WorkbenchTerminalInputEvent::InsertText("hello[<35;55;37m".into())
        ),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(state.buffer(), "hello");
}

#[test]
fn terminal_input_reducer_drops_double_open_mouse_fragments_from_buffer() {
    let mut state = WorkbenchInputState::default();

    assert_eq!(
        apply_workbench_terminal_input_event(
            &mut state,
            WorkbenchTerminalInputEvent::InsertText("[[<35;55;37M".into())
        ),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(state.buffer(), "");

    assert_eq!(
        apply_workbench_terminal_input_event(
            &mut state,
            WorkbenchTerminalInputEvent::InsertText("hello[[<35;55;37m".into())
        ),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(state.buffer(), "hello");
}

#[test]
fn terminal_input_reducer_hides_partial_mouse_tail_from_buffer() {
    let mut state = WorkbenchInputState::default();

    assert_eq!(
        apply_workbench_terminal_input_event(
            &mut state,
            WorkbenchTerminalInputEvent::InsertText("hello[<35;55".into())
        ),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(state.buffer(), "hello");
}

#[test]
fn terminal_paste_event_inserts_redacted_single_line_text() {
    let mut state = WorkbenchInputState::default();
    let event = super::parse_workbench_terminal_event(CrosstermEvent::Paste(
        "hello\napi_key=sk-secret-value\tworld".into(),
    ))
    .expect("paste event");

    assert_eq!(
        apply_workbench_terminal_input_event(&mut state, event),
        WorkbenchTerminalInputOutcome::Pending
    );
    assert_eq!(state.buffer(), "hello_[REDACTED_SECRET]_world");
    assert_eq!(
        apply_workbench_terminal_input_event(&mut state, WorkbenchTerminalInputEvent::Submit),
        WorkbenchTerminalInputOutcome::Submit("hello_[REDACTED_SECRET]_world".into())
    );
}

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use mdtui_tui::{App, handle_event};

#[test]
fn direct_editing_round_trips_to_markdown_source() {
    let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
    handle_event(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Char('#'), KeyModifiers::empty())),
    );
    handle_event(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty())),
    );
    handle_event(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::empty())),
    );

    assert_eq!(app.current().document.source(), "# T");
    assert!(
        app.current()
            .document
            .components
            .components
            .iter()
            .any(|component| matches!(
                component.kind,
                mdtui_core::ComponentKind::Heading { level: 1 }
            ))
    );
}

use super::*;

fn key(code: KeyCode, kind: KeyEventKind) -> Event {
    Event::Key(KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind))
}

#[test]
fn composer_input_batch_stops_before_frame_dependent_input() {
    let press = key(KeyCode::Char('a'), KeyEventKind::Press);
    let release = key(KeyCode::Char('a'), KeyEventKind::Release);
    let next = key(KeyCode::Char('b'), KeyEventKind::Press);
    for boundary in [
        key(KeyCode::Enter, KeyEventKind::Press),
        Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        Event::Paste("bracketed paste".into()),
        Event::Resize(80, 24),
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: 5,
            modifiers: KeyModifiers::NONE,
        }),
    ] {
        let mut events = VecDeque::from([release.clone(), boundary.clone(), next.clone()]);
        let mut pending = None;
        let mut dispatched = Vec::new();
        dispatch_terminal_batch(
            press.clone(),
            &mut pending,
            true,
            || Ok(events.pop_front()),
            |event| dispatched.push(event),
        )
        .expect("dispatch");
        assert_eq!(dispatched, [press.clone(), release.clone()]);
        assert_eq!(pending, Some(boundary));
        assert_eq!(events.pop_front(), Some(next.clone()));
        assert!(events.is_empty());
    }
}

#[test]
fn composer_input_batch_yields_to_the_next_frame_at_the_limit() {
    let press = key(KeyCode::Char('a'), KeyEventKind::Press);
    let mut reads = 0;
    let mut dispatched = 0;
    let mut pending = None;
    dispatch_terminal_batch(
        press.clone(),
        &mut pending,
        true,
        || {
            reads += 1;
            Ok(Some(press.clone()))
        },
        |_| dispatched += 1,
    )
    .expect("dispatch");
    assert_eq!(dispatched, TERMINAL_INPUTS_PER_FRAME);
    assert_eq!(reads, TERMINAL_INPUTS_PER_FRAME - 1);
    assert!(pending.is_none());
}

#[test]
fn character_input_outside_the_composer_keeps_its_own_frame() {
    let press = key(KeyCode::Char('a'), KeyEventKind::Press);
    let mut pending = None;
    let mut dispatched = Vec::new();
    dispatch_terminal_batch(
        press.clone(),
        &mut pending,
        false,
        || panic!("outside the composer, dispatch one input per frame"),
        |event| dispatched.push(event),
    )
    .expect("dispatch");
    assert_eq!(dispatched, [press]);
    assert!(pending.is_none());
}

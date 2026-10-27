use super::*;
use makepad_widgets::makepad_platform::event::{
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, TouchPoint, TouchUpdateEvent,
};
use makepad_widgets::{dvec2, Area, WindowId};

fn rect() -> Rect {
    Rect {
        pos: dvec2(40., 60.),
        size: dvec2(300., 400.),
    }
}
fn down(x: f64, y: f64) -> Event {
    Event::MouseDown(MouseDownEvent {
        abs: dvec2(x, y),
        time: 1.,
        button: MouseButton::PRIMARY,
        window_id: WindowId(0, 0),
        modifiers: Default::default(),
        handled: std::cell::Cell::new(Area::Empty),
    })
}
fn moved(x: f64, y: f64) -> Event {
    Event::MouseMove(MouseMoveEvent {
        abs: dvec2(x, y),
        time: 1.1,
        window_id: WindowId(0, 0),
        modifiers: Default::default(),
        handled: std::cell::Cell::new(Area::Empty),
    })
}
fn up(x: f64, y: f64) -> Event {
    Event::MouseUp(MouseUpEvent {
        abs: dvec2(x, y),
        time: 1.2,
        button: MouseButton::PRIMARY,
        window_id: WindowId(0, 0),
        modifiers: Default::default(),
    })
}
fn touch(uid: u64, state: TouchState, x: f64, y: f64) -> Event {
    Event::TouchUpdate(TouchUpdateEvent {
        time: 1.,
        window_id: WindowId(0, 0),
        modifiers: Default::default(),
        touches: vec![TouchPoint {
            state,
            abs: dvec2(x, y),
            time: 1.,
            uid,
            rotation_angle: 0.,
            force: 1.,
            radius: dvec2(1., 1.),
            handled: Default::default(),
            sweep_lock: Default::default(),
        }],
    })
}

#[test]
fn native_thumb_drag_keeps_ownership_outside_the_reader_and_forwards_release() {
    let mut p = ScrollbarPointer::default();
    assert!(p.track(&down(335., 100.), rect()));
    assert!(p.track(&moved(120., 600.), rect()));
    assert!(p.track(&up(-30., 600.), rect()));
    assert!(!p.track(&moved(335., 200.), rect()));
    assert!(!p.track(&up(335., 200.), rect()));
    assert!(!p.track(&down(329., 100.), rect()));
    assert!(
        !p.track(&moved(335., 100.), rect()),
        "text drag entering rail is not a thumb drag"
    );
    assert!(!p.track(&down(335., 20.), rect()));
}

#[test]
fn touch_thumb_drag_ends_only_with_its_own_release_and_cannot_toggle_toolbar() {
    let mut p = ScrollbarPointer::default();
    assert!(p.track(&touch(1, TouchState::Start, 335., 100.), rect()));
    assert!(p.track(&touch(2, TouchState::Start, 100., 100.), rect()));
    assert!(p.track(&touch(2, TouchState::Stop, 100., 100.), rect()));
    assert!(p.track(&touch(1, TouchState::Move, -100., 100.), rect()));
    assert!(p.track(&touch(1, TouchState::Stop, -100., 100.), rect()));
    assert!(!p.track(&touch(2, TouchState::Move, 335., 100.), rect()));
    assert!(!p.track(&touch(3, TouchState::Start, 150., 100.), rect()));
    assert!(!p.track(&touch(3, TouchState::Move, 335., 100.), rect()));
}

#[test]
fn reload_and_document_reset_do_not_leave_a_scrollbar_pointer_latched() {
    let mut p = ScrollbarPointer::default();
    for reset in [Event::LiveEdit, Event::ScriptReapply] {
        assert!(p.track(&down(335., 100.), rect()));
        assert!(!p.track(&reset, rect()));
        assert!(!p.track(&moved(335., 150.), rect()));
    }
    p.track(&touch(1, TouchState::Start, 335., 100.), rect());
    p.cancel();
    assert!(!p.track(&touch(1, TouchState::Move, 335., 200.), rect()));
}

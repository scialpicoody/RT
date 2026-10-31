//! Winit `WindowEvent` translation, through the public seam:
//! `InputTranslator::translate`. Pure: constructing winit event values opens
//! no window, so these tests run headless.

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use rt_gui::{InputEvent, InputTranslator};

fn moved(x: f64, y: f64) -> WindowEvent {
    WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    }
}

fn click(state: ElementState, button: MouseButton) -> WindowEvent {
    WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state,
        button,
    }
}

#[test]
fn cursor_moves_translate_to_physical_pixel_coordinates() {
    let mut t = InputTranslator::new();
    assert_eq!(
        t.translate(&moved(10.0, 20.0)),
        Some(InputEvent::MouseMoved { x: 10, y: 20 })
    );
    assert_eq!(
        t.translate(&moved(3.7, 9.9)),
        Some(InputEvent::MouseMoved { x: 3, y: 9 })
    );
}

#[test]
fn mouse_press_and_release_carry_the_last_cursor_position() {
    let mut t = InputTranslator::new();
    t.translate(&moved(40.0, 30.0));
    assert_eq!(
        t.translate(&click(ElementState::Pressed, MouseButton::Left)),
        Some(InputEvent::MousePressed { x: 40, y: 30 })
    );
    assert_eq!(
        t.translate(&click(ElementState::Released, MouseButton::Left)),
        Some(InputEvent::MouseReleased { x: 40, y: 30 })
    );
}

#[test]
fn press_without_prior_cursor_position_is_dropped() {
    let mut t = InputTranslator::new();
    assert_eq!(
        t.translate(&click(ElementState::Pressed, MouseButton::Left)),
        None
    );
}

#[test]
fn close_and_resize_translate_directly() {
    let mut t = InputTranslator::new();
    assert_eq!(t.translate(&WindowEvent::CloseRequested), Some(InputEvent::CloseRequested));
    assert_eq!(
        t.translate(&WindowEvent::Resized(PhysicalSize::new(640, 480))),
        Some(InputEvent::Resized { width: 640, height: 480 })
    );
}

#[test]
fn untranslated_window_events_yield_none() {
    let mut t = InputTranslator::new();
    assert_eq!(t.translate(&WindowEvent::Focused(true)), None);
    assert_eq!(t.translate(&WindowEvent::CursorLeft { device_id: DeviceId::dummy() }), None);
}

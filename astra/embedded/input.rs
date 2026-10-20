use super::{abi, Result};
use siglus_scene_vm::{
    host::SiglusHost,
    runtime::input::{VmKey, VmMouseButton},
};

const MAX_TEXT_INPUT_BYTES: usize = 32 * 1024;

pub(super) enum Event {
    Key {
        code: i32,
        text: Option<String>,
        pressed: bool,
        repeat: bool,
    },
    Text(String),
    Move(f64, f64),
    Button(VmMouseButton, bool),
    Wheel(i32),
    Window(abi::WindowState),
}
pub(super) fn valid_window(window: abi::WindowState) -> bool {
    window.focused <= 1
        && window.suspended <= 1
        && window.scale_factor.is_finite()
        && window.scale_factor > 0.0
}
pub(super) unsafe fn decode(event: &abi::InputEvent, epoch: u64, generation: u64) -> Result<Event> {
    if event.pressed > 1 || event.repeat > 1 {
        return Err(abi::INVALID_ARGUMENT.into());
    }
    if matches!(
        event.kind,
        abi::INPUT_POINTER_MOVE | abi::INPUT_POINTER_BUTTON | abi::INPUT_WHEEL
    ) && (event.displayed.session_epoch != epoch
        || event.displayed.output_epoch != 1
        || event.displayed.generation == 0
        || event.displayed.generation > generation)
    {
        return Err(abi::STALE_CONTEXT.into());
    }
    Ok(match event.kind {
        abi::INPUT_WINDOW => {
            if !valid_window(event.window) {
                return Err(abi::INVALID_ARGUMENT.into());
            }
            Event::Window(event.window)
        }
        abi::INPUT_KEY => {
            let physical = unsafe { event.physical_key.text(128)? };
            if physical.is_empty() {
                let text = unsafe { event.logical_key.text(MAX_TEXT_INPUT_BYTES)? };
                if event.pressed != 1 || event.repeat != 0 || text.is_empty() {
                    return Err(abi::INVALID_ARGUMENT.into());
                }
                return Ok(Event::Text(text.to_owned()));
            }
            let logical = unsafe { event.logical_key.text(128)? };
            let code = key_code(physical).ok_or(abi::UNSUPPORTED)?;
            let text = if logical.chars().count() == 1 && !logical.chars().any(char::is_control) {
                Some(logical.to_owned())
            } else {
                None
            };
            Event::Key {
                code,
                text,
                pressed: event.pressed != 0,
                repeat: event.repeat != 0,
            }
        }
        abi::INPUT_POINTER_MOVE => {
            if !event.x.is_finite() || !event.y.is_finite() {
                return Err(abi::INVALID_ARGUMENT.into());
            }
            Event::Move(event.x, event.y)
        }
        abi::INPUT_POINTER_BUTTON => Event::Button(
            match event.button {
                1 => VmMouseButton::Left,
                2 => VmMouseButton::Right,
                3 => VmMouseButton::Middle,
                _ => return Err(abi::UNSUPPORTED.into()),
            },
            event.pressed != 0,
        ),
        abi::INPUT_WHEEL => {
            if !event.y.is_finite() || event.y.abs() > i32::MAX as f64 {
                return Err(abi::INVALID_ARGUMENT.into());
            }
            Event::Wheel(event.y.round() as i32)
        }
        _ => return Err(abi::UNSUPPORTED.into()),
    })
}
impl Event {
    pub fn apply(self, host: &mut SiglusHost) {
        match self {
            Self::Key {
                code: code @ 0x70..=0x7b,
                pressed,
                ..
            } => {
                let key = VmKey::F((code - 0x6f) as u8);
                if pressed {
                    host.key_down(key);
                } else {
                    host.key_up(key);
                }
            }
            Self::Key {
                code,
                text,
                pressed: true,
                repeat,
            } => host.key_event(code, text.as_deref(), repeat),
            Self::Key {
                code,
                pressed: false,
                ..
            } => host.key_up_code(code),
            Self::Text(text) => host.text_input(&text),
            Self::Move(x, y) => host.mouse_move(x, y),
            Self::Button(button, true) => host.mouse_down(button),
            Self::Button(button, false) => host.mouse_up(button),
            Self::Wheel(delta) => host.mouse_wheel(delta),
            Self::Window(_) => unreachable!("window is handled by session lifecycle"),
        }
    }
}
fn key_code(key: &str) -> Option<i32> {
    Some(match key {
        "Escape" => 0x1b,
        "Enter" | "NumpadEnter" => 0x0d,
        "Space" => 0x20,
        "Backspace" => 8,
        "Delete" => 0x2e,
        "Tab" => 9,
        "ShiftLeft" | "ShiftRight" => 0x10,
        "ControlLeft" | "ControlRight" => 0x11,
        "AltLeft" | "AltRight" => 0x12,
        "MetaLeft" | "MetaRight" => 0x5b,
        "Home" => 0x24,
        "End" => 0x23,
        "ArrowLeft" => 0x25,
        "ArrowUp" => 0x26,
        "ArrowRight" => 0x27,
        "ArrowDown" => 0x28,
        // The desktop core routes these unmapped physical keys through
        // key_event's wait-key/direct-text handling, not an unsupported error.
        "Insert" | "PageUp" | "PageDown" | "Numpad0" | "Numpad1" | "Numpad2" | "Numpad3"
        | "Numpad4" | "Numpad5" | "Numpad6" | "Numpad7" | "Numpad8" | "Numpad9" | "NumpadAdd"
        | "NumpadSubtract" | "NumpadMultiply" | "NumpadDivide" | "Minus" | "Equal" | "Comma"
        | "Period" | "Slash" | "Semicolon" | "Quote" | "BracketLeft" | "BracketRight"
        | "Backslash" | "Backquote" => -1,
        _ => {
            if let Some(letter) = key
                .strip_prefix("Key")
                .filter(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_uppercase())
            {
                i32::from(letter.as_bytes()[0])
            } else if let Some(digit) = key
                .strip_prefix("Digit")
                .filter(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_digit())
            {
                i32::from(digit.as_bytes()[0])
            } else {
                let n = key
                    .strip_prefix('F')
                    .and_then(|s| s.parse::<i32>().ok())
                    .filter(|n| (1..=12).contains(n))?;
                0x6f + n
            }
        }
    })
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;

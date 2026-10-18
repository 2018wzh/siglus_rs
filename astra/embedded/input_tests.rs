use super::*;

fn key(physical: &str, logical: &str) -> abi::InputEvent {
    abi::InputEvent {
        kind: abi::INPUT_KEY,
        pressed: 1,
        repeat: 0,
        button: 0,
        physical_key: abi::Bytes::new(physical.as_bytes()),
        logical_key: abi::Bytes::new(logical.as_bytes()),
        x: 0.0,
        y: 0.0,
        displayed: abi::FrameIdentity {
            session_epoch: 1,
            output_epoch: 1,
            generation: 1,
        },
        window: abi::WindowState {
            drawable_width: 1280,
            drawable_height: 720,
            scale_factor: 1.0,
            focused: 1,
            suspended: 0,
        },
    }
}

fn status(event: &abi::InputEvent) -> abi::Status {
    match unsafe { decode(event, 1, 1) } {
        Ok(_) => abi::OK,
        Err(failure) => failure.status,
    }
}

#[test]
fn desktop_unmapped_keys_reach_native_wait_and_text_path() {
    for physical in [
        "Insert", "PageUp", "PageDown", "Numpad0", "Numpad1", "Numpad2", "Numpad3",
        "Numpad4", "Numpad5", "Numpad6", "Numpad7", "Numpad8", "Numpad9", "NumpadAdd",
        "NumpadSubtract", "NumpadMultiply", "NumpadDivide", "Minus", "Equal", "Comma",
        "Period", "Slash", "Semicolon", "Quote", "BracketLeft", "BracketRight", "Backslash",
        "Backquote",
    ] {
        for pressed in [0, 1] {
            let mut event = key(physical, ";");
            event.pressed = pressed;
            event.repeat = pressed;
            let Ok(Event::Key {
                code,
                text,
                pressed: decoded_pressed,
                repeat,
            }) = (unsafe { decode(&event, 1, 1) }) else {
                panic!("native key {physical} rejected");
            };
            assert_eq!(code, -1);
            assert_eq!(text.as_deref(), Some(";"));
            assert_eq!(decoded_pressed, pressed != 0);
            assert_eq!(repeat, pressed != 0);
        }
    }
}

#[test]
fn native_enter_and_meta_aliases_keep_their_virtual_keys() {
    for (physical, expected) in [
        ("Enter", 0x0d),
        ("NumpadEnter", 0x0d),
        ("MetaLeft", 0x5b),
        ("MetaRight", 0x5b),
        ("F1", 0x70),
        ("F12", 0x7b),
        ("KeyA", 0x41),
        ("Digit0", 0x30),
    ] {
        let Ok(Event::Key { code, .. }) = (unsafe { decode(&key(physical, ""), 1, 1) }) else {
            panic!("native key {physical} rejected");
        };
        assert_eq!(code, expected);
    }
}

#[test]
fn committed_text_is_not_a_physical_key_or_single_character() {
    let text = "日本語入力".repeat(100);
    let Ok(Event::Text(decoded)) = (unsafe { decode(&key("", &text), 1, 1) }) else {
        panic!("committed text rejected");
    };
    assert_eq!(decoded, text);
    let text = "a".repeat(MAX_TEXT_INPUT_BYTES);
    assert_eq!(status(&key("", &text)), abi::OK);
    let oversized = "a".repeat(MAX_TEXT_INPUT_BYTES + 1);
    assert_eq!(status(&key("", &oversized)), abi::INVALID_ARGUMENT);
}

#[test]
fn malformed_committed_text_is_rejected() {
    for text in ["", "a\0b"] {
        assert_eq!(status(&key("", text)), abi::INVALID_ARGUMENT);
    }
    for (pressed, repeat) in [(0, 0), (1, 1), (2, 0)] {
        let mut event = key("", "text");
        event.pressed = pressed;
        event.repeat = repeat;
        assert_eq!(status(&event), abi::INVALID_ARGUMENT);
    }
    let mut event = key("", "text");
    event.logical_key = abi::Bytes::new(&[0xff]);
    assert_eq!(status(&event), abi::INVALID_ARGUMENT);
}

#[test]
fn unknown_inputs_and_stale_pointer_frames_remain_errors() {
    for physical in ["Unknown", "Unidentified", "KeyAA", "Keya", "Digit10", "F0", "F13"] {
        assert_eq!(status(&key(physical, "a")), abi::UNSUPPORTED);
    }
    let mut event = key("", "");
    event.kind = abi::INPUT_POINTER_BUTTON;
    event.button = 4;
    assert_eq!(status(&event), abi::UNSUPPORTED);
    event.button = abi::BUTTON_PRIMARY;
    event.displayed.session_epoch = 2;
    assert_eq!(status(&event), abi::STALE_CONTEXT);
    event.kind = u32::MAX;
    assert_eq!(status(&event), abi::UNSUPPORTED);
    assert_eq!(status(&key("KeyA", "a")), abi::OK);
}

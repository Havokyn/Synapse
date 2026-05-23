use synapse_action::{ResolvedBackend, resolve_backend};
use synapse_core::{Action, Backend, ButtonAction, Key, KeyCode, MouseButton, PadButton};

#[test]
fn explicit_backend_variants_resolve_or_fail_closed() {
    let action = key_down_action();

    assert_backend(
        "explicit_software",
        Backend::Software,
        &action,
        ResolvedBackend::Software,
    );
    assert_backend(
        "explicit_vigem",
        Backend::Vigem,
        &action,
        ResolvedBackend::Vigem,
    );

    assert_backend(
        "explicit_hardware",
        Backend::Hardware,
        &action,
        ResolvedBackend::Hardware,
    );

    assert_backend(
        "explicit_auto_keyboard",
        Backend::Auto,
        &action,
        ResolvedBackend::Software,
    );
}

#[test]
fn auto_backend_routes_keyboard_mouse_and_pad_actions() {
    assert_backend(
        "auto_keyboard",
        Backend::Auto,
        &key_down_action(),
        ResolvedBackend::Software,
    );
    assert_backend(
        "auto_mouse",
        Backend::Auto,
        &mouse_button_action(),
        ResolvedBackend::Software,
    );
    assert_backend(
        "auto_pad",
        Backend::Auto,
        &pad_button_action(),
        ResolvedBackend::Vigem,
    );
}

fn assert_backend(edge: &str, requested: Backend, action: &Action, expected: ResolvedBackend) {
    let resolved = resolve_backend(requested, action)
        .unwrap_or_else(|err| panic!("{edge} should resolve backend, got {err}"));
    assert_eq!(resolved, expected);
    println!(
        "source_of_truth=backend_resolution edge={edge} before_backend={requested:?} after_backend={} final_value={:?}",
        resolved.as_str(),
        resolved
    );
}

fn key_down_action() -> Action {
    Action::KeyDown {
        key: Key {
            code: KeyCode::Named {
                value: "a".to_owned(),
            },
            use_scancode: false,
        },
        backend: Backend::Auto,
    }
}

const fn mouse_button_action() -> Action {
    Action::MouseButton {
        button: MouseButton::Left,
        action: ButtonAction::Down,
        hold_ms: 0,
        backend: Backend::Auto,
    }
}

const fn pad_button_action() -> Action {
    Action::PadButton {
        pad: 0,
        button: PadButton::A,
        action: ButtonAction::Down,
        hold_ms: 0,
    }
}

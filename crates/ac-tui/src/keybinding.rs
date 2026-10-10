//! Configurable keybinding matcher and event string formatting.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Format a crossterm `KeyEvent` into a human-readable keybinding string.
/// Examples: `"Ctrl+Q"`, `"Alt+S"`, `"Ctrl+\\"`, `"F1"`, `"n"`, `"?"`.
pub fn key_event_to_string(key: &KeyEvent) -> String {
    let mut parts = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt");
    }

    let code_str = match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_alphabetic() {
                parts.push("Shift");
                c.to_ascii_uppercase().to_string()
            } else if c.is_ascii_alphabetic() && !parts.is_empty() {
                c.to_ascii_uppercase().to_string()
            } else {
                c.to_string()
            }
        }
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => {
            parts.push("Shift");
            "Tab".to_string()
        }
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::PageUp => "PgUp".to_string(),
        KeyCode::PageDown => "PgDn".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        _ => return String::new(),
    };

    if parts.is_empty() {
        return code_str;
    }

    parts.push(&code_str);
    parts.join("+")
}

/// Check if a `KeyEvent` matches the specified keybinding string (e.g. `"Ctrl+Q"`, `"q"`, `"?"`).
pub fn matches_key(key: &KeyEvent, spec: &str) -> bool {
    let spec = spec.trim();
    if spec.is_empty() {
        return false;
    }

    // Split on '+' except when '+' is the key itself at the end (e.g. "Ctrl++")
    let (mod_tokens, key_token) = if spec == "+" || spec == "++" {
        (Vec::new(), "+")
    } else if spec.ends_with("++") {
        let prefix = &spec[..spec.len() - 2];
        (prefix.split('+').collect::<Vec<_>>(), "+")
    } else {
        let mut tokens: Vec<&str> = spec.split('+').collect();
        let k = tokens.pop().unwrap_or("");
        (tokens, k)
    };

    let mut need_ctrl = false;
    let mut need_alt = false;
    let mut need_shift = false;

    for m in mod_tokens {
        match m.to_lowercase().as_str() {
            "ctrl" | "control" => need_ctrl = true,
            "alt" => need_alt = true,
            "shift" => need_shift = true,
            _ => {}
        }
    }

    if need_ctrl != key.modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    if need_alt != key.modifiers.contains(KeyModifiers::ALT) {
        return false;
    }
    if need_shift && !key.modifiers.contains(KeyModifiers::SHIFT) {
        return false;
    }

    match key_token.to_lowercase().as_str() {
        "esc" | "escape" => key.code == KeyCode::Esc,
        "enter" | "return" => key.code == KeyCode::Enter,
        "tab" => key.code == KeyCode::Tab,
        "backtab" => key.code == KeyCode::BackTab,
        "backspace" => key.code == KeyCode::Backspace,
        "delete" | "del" => key.code == KeyCode::Delete,
        "up" => key.code == KeyCode::Up,
        "down" => key.code == KeyCode::Down,
        "left" => key.code == KeyCode::Left,
        "right" => key.code == KeyCode::Right,
        "pgup" | "pageup" => key.code == KeyCode::PageUp,
        "pgdn" | "pagedown" => key.code == KeyCode::PageDown,
        "home" => key.code == KeyCode::Home,
        "end" => key.code == KeyCode::End,
        s if s.starts_with('f') && s.len() > 1 && s[1..].chars().all(|c| c.is_ascii_digit()) => {
            if let Ok(num) = s[1..].parse::<u8>() {
                key.code == KeyCode::F(num)
            } else {
                false
            }
        }
        s if s.chars().count() == 1 => {
            let target_c = s.chars().next().unwrap();
            match key.code {
                KeyCode::Char(c) => c.eq_ignore_ascii_case(&target_c),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Check if a `KeyEvent` triggers the named action according to user settings.
pub fn matches_action(
    key: &KeyEvent,
    action: &str,
    settings: &ac_core::settings::UserSettings,
) -> bool {
    let spec = settings.keybindings.get(action);
    if matches_key(key, spec) {
        return true;
    }
    // Built-in secondary alias fallbacks
    match action {
        "toggle_split" => {
            if spec == "Alt+S"
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('\\')
            {
                return true;
            }
        }
        "command_palette" => {
            if spec == "Ctrl+P" && key.code == KeyCode::F(1) {
                return true;
            }
        }
        "resume_session" => {
            if spec == "Space" && matches!(key.code, KeyCode::Char('u') | KeyCode::Char('U')) {
                return true;
            }
        }
        "remove_session" => {
            if spec == "d" && key.code == KeyCode::Delete {
                return true;
            }
        }
        _ => {}
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_event_formatting() {
        assert_eq!(
            key_event_to_string(&KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            "Ctrl+Q"
        );
        assert_eq!(
            key_event_to_string(&KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT)),
            "Alt+S"
        );
        assert_eq!(
            key_event_to_string(&KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
            "?"
        );
        assert_eq!(
            key_event_to_string(&KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)),
            "F1"
        );
    }

    #[test]
    fn test_matches_key() {
        let ctrl_q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert!(matches_key(&ctrl_q, "Ctrl+Q"));
        assert!(matches_key(&ctrl_q, "ctrl+q"));
        assert!(!matches_key(&ctrl_q, "Ctrl+W"));
        assert!(!matches_key(&ctrl_q, "q"));

        let single_q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(matches_key(&single_q, "q"));
        assert!(matches_key(&single_q, "Q"));
        assert!(!matches_key(&single_q, "Ctrl+Q"));

        let question = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE);
        assert!(matches_key(&question, "?"));

        let alt_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT);
        assert!(matches_key(&alt_s, "Alt+S"));
    }
}

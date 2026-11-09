//! ui/keys.rs — key definitions and Magit-compatible key sequences.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A parsed key event, normalized for comparison (case-sensitive letters,
/// Ctrl combos recorded separately).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: String, // "a", "A", "tab", "enter", "esc", "g" ...
    pub ctrl: bool,
}

impl Key {
    pub fn from_event(ev: &KeyEvent) -> Key {
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        let code = match ev.code {
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Tab => "tab".into(),
            KeyCode::Enter => "enter".into(),
            KeyCode::Esc => "esc".into(),
            KeyCode::Backspace => "backspace".into(),
            KeyCode::Up => "up".into(),
            KeyCode::Down => "down".into(),
            KeyCode::Left => "left".into(),
            KeyCode::Right => "right".into(),
            KeyCode::Home => "home".into(),
            KeyCode::End => "end".into(),
            KeyCode::PageUp => "pgup".into(),
            KeyCode::PageDown => "pgdn".into(),
            KeyCode::F(n) => format!("f{n}"),
            _ => String::new(),
        };
        Key { code, ctrl }
    }

    /// Display like Emacs: "C-c", "RET", "TAB", "a".
    pub fn label(&self) -> String {
        if self.ctrl {
            format!("C-{}", self.code)
        } else {
            match self.code.as_str() {
                "enter" => "RET".into(),
                "tab" => "TAB".into(),
                "esc" => "ESC".into(),
                other => other.to_string(),
            }
        }
    }
}

/// Translate an Emacs-style key chord string ("C-c", "RET", "a") into Key.
pub fn key_from_label(label: &str) -> Key {
    match label {
        "RET" | "<return>" | "return" => Key { code: "enter".into(), ctrl: false },
        "TAB" | "<tab>" | "tab" => Key { code: "tab".into(), ctrl: false },
        "ESC" | "esc" | "C-[" => Key { code: "esc".into(), ctrl: false },
        "SPC" | " " => Key { code: " ".into(), ctrl: false },
        l if l.starts_with("C-") => Key { code: l[2..].to_lowercase(), ctrl: true },
        // Emacs function-key notation: <up> <down> <left> <right> ...
        l if l.len() > 2 && l.starts_with('<') && l.ends_with('>') => Key {
            code: l[1..l.len() - 1].to_string(),
            ctrl: false,
        },
        l => Key { code: l.to_string(), ctrl: false },
    }
}

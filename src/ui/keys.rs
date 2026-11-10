//! ui/keys.rs — key definitions and Magit-compatible key sequences.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A parsed key event, normalized for comparison (case-sensitive letters,
/// Ctrl combos recorded separately, Meta as an "M-" prefix of the code).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: String, // "a", "A", "tab", "enter", "esc", "g", "M-n" ...
    pub ctrl: bool,
}

impl Key {
    pub fn from_event(ev: &KeyEvent) -> Key {
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        let meta = ev.modifiers.contains(KeyModifiers::ALT);
        let shift = ev.modifiers.contains(KeyModifiers::SHIFT);
        let code = match ev.code {
            // C-@ arrives as NUL, i.e. C-SPC
            KeyCode::Char('@') if ctrl => " ".into(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Tab if shift => "backtab".into(),
            KeyCode::BackTab => "backtab".into(),
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
        let code = if meta && !code.is_empty() { format!("M-{code}") } else { code };
        Key { code, ctrl }
    }

    /// Display like Emacs: "C-c", "RET", "TAB", "a".
    pub fn label(&self) -> String {
        if self.ctrl {
            match self.code.as_str() {
                " " => "C-SPC".into(),
                c => format!("C-{c}"),
            }
        } else {
            match self.code.as_str() {
                "enter" => "RET".into(),
                "tab" => "TAB".into(),
                "esc" => "ESC".into(),
                " " => "SPC".into(),
                "backtab" => "<backtab>".into(),
                other => other.to_string(),
            }
        }
    }
}

/// Translate an Emacs-style key chord string ("C-c", "RET", "a", "M-n",
/// "C-SPC", "<backtab>") into Key.
pub fn key_from_label(label: &str) -> Key {
    match label {
        // a terminal sends C-m as RET, as Emacs translates it
        "RET" | "<return>" | "return" | "C-m" => Key { code: "enter".into(), ctrl: false },
        "TAB" | "<tab>" | "tab" => Key { code: "tab".into(), ctrl: false },
        "ESC" | "esc" | "C-[" => Key { code: "esc".into(), ctrl: false },
        "SPC" | " " => Key { code: " ".into(), ctrl: false },
        "DEL" => Key { code: "backspace".into(), ctrl: false },
        "C-@" => Key { code: " ".into(), ctrl: true },
        l if l.len() > 2 && l.starts_with("M-") => {
            let k = key_from_label(&l[2..]);
            Key { code: format!("M-{}", k.code), ctrl: k.ctrl }
        }
        l if l.len() > 2 && l.starts_with("C-") => {
            let k = key_from_label(&l[2..]);
            let code = if k.code.chars().count() == 1 { k.code.to_lowercase() } else { k.code };
            Key { code, ctrl: true }
        }
        // Emacs function-key notation: <up> <down> <left> <right> ...
        l if l.len() > 2 && l.starts_with('<') && l.ends_with('>') => Key {
            code: l[1..l.len() - 1].to_string(),
            ctrl: false,
        },
        l => Key { code: l.to_string(), ctrl: false },
    }
}

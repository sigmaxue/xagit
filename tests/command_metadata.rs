//! The command names shown for key bindings and menus must not drift from
//! the documented Magit names (and from `ui::help`'s prefix commands).

use xagit_lib::ui::app::{binding_command, status_binding, Binding};
use xagit_lib::ui::keys::key_from_label;

/// `Binding::Menu` used to derive its name with a bare
/// `trim_end_matches("-menu")`, which disagreed with `help::prefix_command`
/// for the menus with special names.
#[test]
fn menu_bindings_have_the_documented_command_names() {
    for (m, want) in [
        ("dispatch", "magit-dispatch"),
        ("file-dispatch", "magit-file-dispatch"),
        ("patches-menu", "magit-am"),
        ("branch-config-menu", "magit-branch-configure"),
        ("margin-settings-menu", "magit-margin-settings"),
        ("mergetool-menu", "magit-git-mergetool"),
        ("commit-menu", "magit-commit"),
        ("log-menu", "magit-log"),
        ("show-refs-menu", "magit-show-refs"),
    ] {
        assert_eq!(binding_command(&Binding::Menu(m)), want, "menu {m}");
    }
}

/// Pin the exceptions to the generic `magit-<short name>` rule.
#[test]
fn command_bindings_keep_the_documented_exceptions() {
    for (c, want) in [
        ("stage-all", "magit-stage-modified"),
        ("jump", "magit-status-jump"),
        ("section-info", "magit-describe-section"),
        ("cherries", "magit-cherry"),
        ("display-buffer", "magit-display-repository-buffer"),
        ("untrack", "magit-file-untrack"),
        ("rename-thing", "magit-file-rename"),
        ("discard", "magit-discard"),
        ("execute-extended-command", "execute-extended-command"),
        ("stage", "magit-stage"),
    ] {
        assert_eq!(binding_command(&Binding::Command(c)), want, "command {c}");
    }
}

#[test]
fn search_keys_are_named_isearch() {
    for (k, want) in [
        ("C-s", "isearch-forward"),
        ("C-r", "isearch-backward"),
        ("C-g", "keyboard-quit"),
    ] {
        let b = status_binding(&key_from_label(k)).unwrap_or_else(|| panic!("no binding for {k}"));
        assert_eq!(binding_command(&b), want, "key {k}");
    }
}

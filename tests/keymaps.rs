//! tests/keymaps.rs — verify every menu matches Magit's key bindings extracted
//! from the lisp sources (magit.el, magit-branch.el, magit-commit.el, ...).

use ngit_lib::ui::transient::{registry, Action};

fn menu(name: &str) -> ngit_lib::ui::transient::Menu {
    registry().get(name).expect("menu exists").clone()
}

fn action_of(m: &ngit_lib::ui::transient::Menu, key: &str) -> Action {
    for g in &m.groups {
        for e in g {
            if e.key == key {
                return e.action.clone();
            }
        }
    }
    panic!("key {key} not found in {}", m.name);
}

/// magit.el:330-403 magit-dispatch
#[test]
fn dispatch_popup_layout() {
    let d = menu("dispatch");
    // dwim/prefix commands column
    assert!(matches!(action_of(&d, "b"), Action::Command(c) if c == "branch-menu"));
    assert!(matches!(action_of(&d, "c"), Action::Command(c) if c == "commit-menu"));
    assert!(matches!(action_of(&d, "f"), Action::Command(c) if c == "fetch-menu"));
    assert!(matches!(action_of(&d, "F"), Action::Command(c) if c == "pull-menu"));
    assert!(matches!(action_of(&d, "P"), Action::Command(c) if c == "push-menu"));
    assert!(matches!(action_of(&d, "m"), Action::Command(c) if c == "merge-menu"));
    assert!(matches!(action_of(&d, "M"), Action::Command(c) if c == "remote-menu"));
    assert!(matches!(action_of(&d, "r"), Action::Command(c) if c == "rebase-menu"));
    assert!(matches!(action_of(&d, "t"), Action::Command(c) if c == "tag-menu"));
    assert!(matches!(action_of(&d, "z"), Action::Command(c) if c == "stash-menu"));
    assert!(matches!(action_of(&d, "X"), Action::Command(c) if c == "reset-menu"));
    assert!(matches!(action_of(&d, "i"), Action::Command(c) if c == "gitignore-menu"));
    assert!(matches!(action_of(&d, "l"), Action::Command(c) if c == "log-menu"));
    assert!(matches!(action_of(&d, "A"), Action::Command(c) if c == "cherry-pick-menu"));
    // applying changes
    assert!(matches!(action_of(&d, "s"), Action::Command(c) if c == "stage"));
    assert!(matches!(action_of(&d, "S"), Action::Command(c) if c == "stage-all"));
    assert!(matches!(action_of(&d, "u"), Action::Command(c) if c == "unstage"));
    assert!(matches!(action_of(&d, "U"), Action::Command(c) if c == "unstage-all"));
    assert!(matches!(action_of(&d, "k"), Action::Command(c) if c == "discard"));
    assert!(matches!(action_of(&d, "V"), Action::Command(c) if c == "revert"));
    assert!(matches!(action_of(&d, "!"), Action::Command(c) if c == "run-menu"));
    // essential
    assert!(matches!(action_of(&d, "g"), Action::Command(c) if c == "refresh"));
    assert!(matches!(action_of(&d, "q"), Action::Command(c) if c == "quit"));
}

/// magit.el:396-401 essential commands + magit-mode.el bindings
#[test]
fn essential_and_mode_map_keys() {
    let d = menu("dispatch");
    assert!(matches!(action_of(&d, "<tab>"), Action::Command(c) if c == "toggle-section"));
    assert!(matches!(action_of(&d, "<return>"), Action::Command(c) if c == "visit"));
    // magit-mode.el:380 h/? open dispatch
    assert!(matches!(action_of(&d, "h"), Action::Command(c) if c == "help"));
}

/// magit-mode.el:370-423 direct bindings map to the same menus
#[test]
fn direct_mode_map_equivalents() {
    // the App key handler mirrors these; menus themselves verified above.
    // Spot-check that every direct key has a corresponding menu entry:
    let reg = registry();
    let pairs = [
        ("branch-menu", "b"),
        ("commit-menu", "c"),
        ("fetch-menu", "u"),   // magit-fetch: fetch from upstream
        ("pull-menu", "u"),    // magit-pull: pull from upstream
        ("push-menu", "u"),    // magit-push: push to upstream
        ("merge-menu", "m"),   // magit-merge: merge
        ("remote-menu", "a"),  // magit-remote: add
        ("rebase-menu", "u"),  // magit-rebase: onto upstream
        ("tag-menu", "t"),     // magit-tag: create tag
        ("stash-menu", "z"),   // magit-stash: stash both
        ("reset-menu", "m"),   // magit-reset: mixed
        ("gitignore-menu", "t"),
        ("log-menu", "l"),
        ("diff-menu", "u"),
        ("cherry-pick-menu", "A"),
    ];
    for (menu_name, key) in pairs {
        let m = reg.get(menu_name).unwrap();
        assert!(matches!(action_of(m, key), Action::Command(_)), "{menu_name} {key}");
    }
    // dispatch popup opens each submenu under Magit's prefix keys
    let d = reg.get("dispatch").unwrap();
    for (menu_name, disp_key) in [
        ("branch-menu", "b"),
        ("commit-menu", "c"),
        ("fetch-menu", "f"),
        ("pull-menu", "F"),
        ("push-menu", "P"),
        ("merge-menu", "m"),
        ("remote-menu", "M"),
        ("rebase-menu", "r"),
        ("tag-menu", "t"),
        ("stash-menu", "z"),
        ("reset-menu", "X"),
        ("gitignore-menu", "i"),
        ("log-menu", "l"),
        ("diff-menu", "d"),
        ("cherry-pick-menu", "A"),
    ] {
        assert!(
            matches!(action_of(&d, disp_key), Action::Command(c) if c == menu_name),
            "dispatch key {disp_key} should open {menu_name}"
        );
    }
}

/// magit-commit.el magit-commit prefix
#[test]
fn commit_popup_layout() {
    let m = menu("commit-menu");
    // arguments (magit-commit.el:334-345)
    assert!(matches!(action_of(&m, "-a"), Action::Switch(s) if s == "--all"));
    assert!(matches!(action_of(&m, "-e"), Action::Switch(s) if s == "--allow-empty"));
    assert!(matches!(action_of(&m, "-v"), Action::Switch(s) if s == "--verbose"));
    assert!(matches!(action_of(&m, "-n"), Action::Switch(s) if s == "--no-verify"));
    assert!(matches!(action_of(&m, "-R"), Action::Switch(s) if s == "--reset-author"));
    // actions (magit-commit.el:346-359)
    assert!(matches!(action_of(&m, "c"), Action::Command(c) if c == "commit-create"));
    assert!(matches!(action_of(&m, "e"), Action::Command(c) if c == "commit-extend"));
    assert!(matches!(action_of(&m, "a"), Action::Command(c) if c == "commit-amend"));
    assert!(matches!(action_of(&m, "w"), Action::Command(c) if c == "commit-reword"));
    assert!(matches!(action_of(&m, "f"), Action::Command(c) if c == "commit-fixup"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "commit-squash"));
    assert!(matches!(action_of(&m, "F"), Action::Command(c) if c == "commit-instant-fixup"));
    assert!(matches!(action_of(&m, "S"), Action::Command(c) if c == "commit-instant-squash"));
}

/// magit-branch.el:215 magit-branch prefix
#[test]
fn branch_popup_layout() {
    let m = menu("branch-menu");
    assert!(matches!(action_of(&m, "b"), Action::Command(c) if c == "checkout"));
    assert!(matches!(action_of(&m, "l"), Action::Command(c) if c == "branch-checkout"));
    assert!(matches!(action_of(&m, "c"), Action::Command(c) if c == "branch-and-checkout"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "branch-spinoff"));
    assert!(matches!(action_of(&m, "n"), Action::Command(c) if c == "branch-create"));
    assert!(matches!(action_of(&m, "S"), Action::Command(c) if c == "branch-spinout"));
    assert!(matches!(action_of(&m, "m"), Action::Command(c) if c == "branch-rename"));
    assert!(matches!(action_of(&m, "x"), Action::Command(c) if c == "branch-reset"));
    assert!(matches!(action_of(&m, "k"), Action::Command(c) if c == "branch-delete"));
}

/// magit-remote.el:66 magit-remote prefix
#[test]
fn remote_popup_layout() {
    let m = menu("remote-menu");
    assert!(matches!(action_of(&m, "-f"), Action::Switch(s) if s == "-f"));
    assert!(matches!(action_of(&m, "a"), Action::Command(c) if c == "remote-add"));
    assert!(matches!(action_of(&m, "r"), Action::Command(c) if c == "remote-rename"));
    assert!(matches!(action_of(&m, "k"), Action::Command(c) if c == "remote-remove"));
    assert!(matches!(action_of(&m, "p"), Action::Command(c) if c == "remote-prune"));
}

/// magit-fetch.el:34 magit-fetch prefix
#[test]
fn fetch_popup_layout() {
    let m = menu("fetch-menu");
    assert!(matches!(action_of(&m, "-p"), Action::Switch(s) if s == "--prune"));
    assert!(matches!(action_of(&m, "-t"), Action::Switch(s) if s == "--tags"));
    assert!(matches!(action_of(&m, "-F"), Action::Switch(s) if s == "--force"));
    assert!(matches!(action_of(&m, "u"), Action::Command(c) if c == "fetch-upstream"));
    assert!(matches!(action_of(&m, "e"), Action::Command(c) if c == "fetch-other"));
    assert!(matches!(action_of(&m, "a"), Action::Command(c) if c == "fetch-all"));
}

/// magit-pull.el:42 magit-pull prefix
#[test]
fn pull_popup_layout() {
    let m = menu("pull-menu");
    assert!(matches!(action_of(&m, "-f"), Action::Switch(s) if s == "--ff-only"));
    assert!(matches!(action_of(&m, "-r"), Action::Switch(s) if s == "--rebase"));
    assert!(matches!(action_of(&m, "-A"), Action::Switch(s) if s == "--autostash"));
    assert!(matches!(action_of(&m, "u"), Action::Command(c) if c == "pull-upstream"));
    assert!(matches!(action_of(&m, "e"), Action::Command(c) if c == "pull-other"));
}

/// magit-push.el magit-push prefix
#[test]
fn push_popup_layout() {
    let m = menu("push-menu");
    assert!(matches!(action_of(&m, "-f"), Action::Switch(s) if s == "--force-with-lease"));
    assert!(matches!(action_of(&m, "-F"), Action::Switch(s) if s == "--force"));
    assert!(matches!(action_of(&m, "-n"), Action::Switch(s) if s == "--dry-run"));
    assert!(matches!(action_of(&m, "-u"), Action::Switch(s) if s == "--set-upstream"));
    assert!(matches!(action_of(&m, "u"), Action::Command(c) if c == "push-upstream"));
    assert!(matches!(action_of(&m, "e"), Action::Command(c) if c == "push-other"));
    assert!(matches!(action_of(&m, "t"), Action::Command(c) if c == "push-tag"));
}

/// magit-merge.el:37 magit-merge prefix
#[test]
fn merge_popup_layout() {
    let m = menu("merge-menu");
    assert!(matches!(action_of(&m, "-f"), Action::Switch(s) if s == "--ff-only"));
    assert!(matches!(action_of(&m, "-n"), Action::Switch(s) if s == "--no-ff"));
    assert!(matches!(action_of(&m, "m"), Action::Command(c) if c == "merge-plain"));
    assert!(matches!(action_of(&m, "n"), Action::Command(c) if c == "merge-nocommit"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "merge-squash"));
    assert!(matches!(action_of(&m, "a"), Action::Command(c) if c == "merge-abort"));
}

/// magit-sequence.el:526 magit-rebase prefix
#[test]
fn rebase_popup_layout() {
    let m = menu("rebase-menu");
    assert!(matches!(action_of(&m, "-a"), Action::Switch(s) if s == "--autosquash"));
    assert!(matches!(action_of(&m, "-A"), Action::Switch(s) if s == "--autostash"));
    assert!(matches!(action_of(&m, "-i"), Action::Switch(s) if s == "--interactive"));
    assert!(matches!(action_of(&m, "-d"), Action::Switch(s) if s == "--committer-date-is-author-date"));
    assert!(matches!(action_of(&m, "-t"), Action::Switch(s) if s == "--ignore-date"));
    assert!(matches!(action_of(&m, "u"), Action::Command(c) if c == "rebase-upstream"));
    assert!(matches!(action_of(&m, "e"), Action::Command(c) if c == "rebase-other"));
    assert!(matches!(action_of(&m, "r"), Action::Command(c) if c == "rebase-continue"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "rebase-skip"));
    assert!(matches!(action_of(&m, "a"), Action::Command(c) if c == "rebase-abort"));
}

/// magit-reset.el:34 magit-reset prefix
#[test]
fn reset_popup_layout() {
    let m = menu("reset-menu");
    assert!(matches!(action_of(&m, "m"), Action::Command(c) if c == "reset-mixed"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "reset-soft"));
    assert!(matches!(action_of(&m, "h"), Action::Command(c) if c == "reset-hard"));
    assert!(matches!(action_of(&m, "k"), Action::Command(c) if c == "reset-keep"));
    assert!(matches!(action_of(&m, "i"), Action::Command(c) if c == "reset-index"));
    assert!(matches!(action_of(&m, "w"), Action::Command(c) if c == "reset-worktree"));
}

/// magit-tag.el:37 magit-tag prefix
#[test]
fn tag_popup_layout() {
    let m = menu("tag-menu");
    assert!(matches!(action_of(&m, "-f"), Action::Switch(s) if s == "--force"));
    assert!(matches!(action_of(&m, "-a"), Action::Switch(s) if s == "--annotate"));
    assert!(matches!(action_of(&m, "-s"), Action::Switch(s) if s == "--sign"));
    assert!(matches!(action_of(&m, "t"), Action::Command(c) if c == "tag-create"));
    assert!(matches!(action_of(&m, "k"), Action::Command(c) if c == "tag-delete"));
}

/// magit-stash.el:90 magit-stash prefix
#[test]
fn stash_popup_layout() {
    let m = menu("stash-menu");
    assert!(matches!(action_of(&m, "-u"), Action::Switch(s) if s == "--include-untracked"));
    assert!(matches!(action_of(&m, "-a"), Action::Switch(s) if s == "--all"));
    assert!(matches!(action_of(&m, "z"), Action::Command(c) if c == "stash-both"));
    assert!(matches!(action_of(&m, "i"), Action::Command(c) if c == "stash-index"));
    assert!(matches!(action_of(&m, "w"), Action::Command(c) if c == "stash-worktree"));
    assert!(matches!(action_of(&m, "x"), Action::Command(c) if c == "stash-keep-index"));
    assert!(matches!(action_of(&m, "Z"), Action::Command(c) if c == "snapshot-both"));
    assert!(matches!(action_of(&m, "I"), Action::Command(c) if c == "snapshot-index"));
    assert!(matches!(action_of(&m, "W"), Action::Command(c) if c == "snapshot-worktree"));
    assert!(matches!(action_of(&m, "a"), Action::Command(c) if c == "stash-apply"));
    assert!(matches!(action_of(&m, "p"), Action::Command(c) if c == "stash-pop"));
    assert!(matches!(action_of(&m, "k"), Action::Command(c) if c == "stash-drop"));
    assert!(matches!(action_of(&m, "l"), Action::Command(c) if c == "stash-list"));
    assert!(matches!(action_of(&m, "v"), Action::Command(c) if c == "stash-show"));
    assert!(matches!(action_of(&m, "b"), Action::Command(c) if c == "stash-branch"));
}

/// magit-gitignore.el magit-gitignore prefix
#[test]
fn gitignore_popup_layout() {
    let m = menu("gitignore-menu");
    assert!(matches!(action_of(&m, "t"), Action::Command(c) if c == "ignore"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "ignore"));
}

/// magit-diff.el magit-diff prefix
#[test]
fn diff_popup_layout() {
    let m = menu("diff-menu");
    assert!(matches!(action_of(&m, "u"), Action::Command(c) if c == "diff-unstaged"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "diff-staged"));
    assert!(matches!(action_of(&m, "c"), Action::Command(c) if c == "show-commit"));
    assert!(matches!(action_of(&m, "t"), Action::Command(c) if c == "stash-show"));
}

/// magit-log.el magit-log prefix
#[test]
fn log_popup_layout() {
    let m = menu("log-menu");
    assert!(matches!(action_of(&m, "l"), Action::Command(c) if c == "log-current"));
    assert!(matches!(action_of(&m, "o"), Action::Command(c) if c == "log-other"));
    assert!(matches!(action_of(&m, "L"), Action::Command(c) if c == "log-branches"));
    assert!(matches!(action_of(&m, "b"), Action::Command(c) if c == "log-all-branches"));
}

/// magit-run prefix ("!")
#[test]
fn run_popup_layout() {
    let m = menu("run-menu");
    assert!(matches!(action_of(&m, "!"), Action::Command(c) if c == "git-command"));
    assert!(matches!(action_of(&m, "p"), Action::Command(c) if c == "git-command"));
}

/// magit-sequence.el:140 magit-cherry-pick prefix
#[test]
fn cherry_pick_popup_layout() {
    let m = menu("cherry-pick-menu");
    assert!(matches!(action_of(&m, "-x"), Action::Switch(s) if s == "-x"));
    assert!(matches!(action_of(&m, "-e"), Action::Switch(s) if s == "--edit"));
    assert!(matches!(action_of(&m, "A"), Action::Command(c) if c == "cherry-pick"));
    assert!(matches!(action_of(&m, "a"), Action::Command(c) if c == "cherry-apply"));
}

/// Key translation mirrors Emacs notation.
#[test]
fn key_labels() {
    use ngit_lib::ui::keys::key_from_label;
    assert_eq!(key_from_label("RET").code, "enter");
    assert_eq!(key_from_label("<tab>").code, "tab");
    assert!(key_from_label("C-g").ctrl);
    assert_eq!(key_from_label("SPC").code, " ");
}

/// bisect/worktree/submodule/notes/ediff are no longer marked unsupported.
#[test]
fn extra_features_popup_layout() {
    let reg = registry();
    let extras = [
        "bisect-menu",
        "worktree-menu",
        "submodule-menu",
        "notes-menu",
        "ediff-menu",
    ];
    for name in extras {
        assert!(reg.contains_key(name), "{name} missing");
        let m = reg.get(name).unwrap();
        assert!(!m.groups.is_empty());
        for e in m.groups.iter().flatten() {
            assert!(
                !matches!(&e.action, Action::Command(c) if c == "unsupported"),
                "{name} key {} still unsupported",
                e.key
            );
        }
    }

    let d = menu("dispatch");
    for (key, target) in [
        ("B", "bisect-menu"),
        ("Z", "worktree-menu"),
        ("o", "submodule-menu"),
        ("T", "notes-menu"),
        ("E", "ediff-menu"),
        ("e", "ediff-dwim"),
        ("D", "diff-change"),
    ] {
        assert!(matches!(action_of(&d, key), Action::Command(c) if c == target), "{key}");
    }

    let b = menu("bisect-menu");
    assert!(matches!(action_of(&b, "B"), Action::Command(c) if c == "bisect-start"));
    assert!(matches!(action_of(&b, "g"), Action::Command(c) if c == "bisect-good"));
    let w = menu("worktree-menu");
    assert!(matches!(action_of(&w, "k"), Action::Command(c) if c == "worktree-delete"));
    let s = menu("submodule-menu");
    assert!(matches!(action_of(&s, "a"), Action::Command(c) if c == "submodule-add"));
    let n = menu("notes-menu");
    assert!(matches!(action_of(&n, "T"), Action::Command(c) if c == "notes-edit"));
    let e = menu("ediff-menu");
    assert!(matches!(action_of(&e, "e"), Action::Command(c) if c == "ediff-dwim"));
}

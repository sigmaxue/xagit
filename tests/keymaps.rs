//! tests/keymaps.rs — verify every menu matches Magit's key bindings extracted
//! from the lisp sources (magit.el, magit-branch.el, magit-commit.el, ...).

use xagit_lib::ui::transient::{registry, resolve_menu, Action, Reader, RepoState};

fn menu(name: &str) -> xagit_lib::ui::transient::Menu {
    registry().get(name).expect("menu exists").clone()
}

fn find(m: &xagit_lib::ui::transient::Menu, key: &str) -> Option<Action> {
    m.groups.iter().flatten().find(|e| e.key == key).map(|e| e.action.clone())
}

fn action_of(m: &xagit_lib::ui::transient::Menu, key: &str) -> Action {
    find(m, key).unwrap_or_else(|| panic!("key {key} not found in {}", m.name))
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
    // magit-dispatch "V" is the magit-revert transient, not a direct revert
    assert!(matches!(action_of(&d, "V"), Action::Command(c) if c == "revert-menu"));
    assert!(matches!(action_of(&d, "a"), Action::Command(c) if c == "apply"));
    assert!(matches!(action_of(&d, "v"), Action::Command(c) if c == "reverse"));
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
    // magit-status-jump; "fu" is two keys as in kbd
    let j = menu("status-jump-menu");
    assert_eq!(j.keys(), ["z", "n", "u", "s", "fu", "fp", "pu", "pp", "j"]);
    assert!(matches!(j.lookup(&xagit_lib::ui::transient::key_seq("f")), xagit_lib::ui::transient::Lookup::Prefix));
    assert!(matches!(action_of(&j, "fu"), Action::Command(c) if c == "jump-to-unpulled-from-upstream"));
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
            matches!(action_of(d, disp_key), Action::Command(c) if c == menu_name),
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
    assert!(matches!(action_of(&m, "A"), Action::Command(c) if c == "commit-alter"));
    assert!(matches!(action_of(&m, "n"), Action::Command(c) if c == "commit-augment"));
    assert!(matches!(action_of(&m, "W"), Action::Command(c) if c == "commit-revise"));
    assert!(matches!(action_of(&m, "F"), Action::Command(c) if c == "commit-instant-fixup"));
    assert!(matches!(action_of(&m, "S"), Action::Command(c) if c == "commit-instant-squash"));
    let level = |k: &str| m.groups.iter().flatten().find(|e| e.key == k).unwrap().level;
    for (key, target, l) in [
        ("d", "commit-reshelve", 0),
        ("R", "rebase-reword-commit", 0),
        ("x", "commit-autofixup", 6),
        ("X", "commit-absorb-modules", 6),
    ] {
        assert!(matches!(action_of(&m, key), Action::Command(c) if c == target), "c {key}");
        assert_eq!(level(key), l, "c {key}");
    }
    // magit-commit-autofixup
    let a = menu("commit-autofixup-menu");
    assert!(matches!(action_of(&a, "-c"), Action::Option { arg, .. } if arg == "--context="));
    assert!(matches!(action_of(&a, "-s"), Action::Option { arg, .. } if arg == "--strict="));
    assert!(matches!(action_of(&a, "-v"), Action::Switch(s) if s == "-vv"));
    assert!(matches!(action_of(&a, "x"), Action::Command(c) if c == "commit-autofixup-run"));
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
    // magit-branch-direct-configure (default t): configuration suffixes
    for (key, target) in [
        ("d", "branch.<branch>.description"),
        ("u", "branch.<branch>.merge/remote"),
        ("r", "branch.<branch>.rebase"),
        ("p", "branch.<branch>.pushRemote"),
        ("R", "pull.rebase"),
        ("P", "remote.pushDefault"),
        ("B", "update-default-branch"),
    ] {
        assert!(matches!(action_of(&m, key), Action::Command(c) if c == target), "b {key}");
    }
}

/// magit-remote.el magit-remote prefix
#[test]
fn remote_popup_layout() {
    let m = menu("remote-menu");
    assert!(matches!(action_of(&m, "-f"), Action::Switch(s) if s == "-f"));
    for (key, target) in [
        ("u", "remote.<remote>.url"),
        ("U", "remote.<remote>.fetch"),
        ("s", "remote.<remote>.pushurl"),
        ("S", "remote.<remote>.push"),
        ("O", "remote.<remote>.tagopt"),
        ("h", "remote.<remote>.followremotehead"),
        ("a", "remote-add"),
        ("r", "remote-rename"),
        ("k", "remote-remove"),
        ("C", "remote-configure-menu"),
        ("p", "remote-prune"),
        ("P", "remote-prune-refspecs"),
        ("z", "remote-unshallow"),
        ("d u", "update-default-branch"),
    ] {
        assert!(matches!(action_of(&m, key), Action::Command(c) if c == target), "M {key}");
    }
    let level = |k: &str| m.groups.iter().flatten().find(|e| e.key == k).unwrap().level;
    assert_eq!(level("z"), 7);
}

/// magit-fetch.el:34 magit-fetch prefix
#[test]
fn fetch_popup_layout() {
    let m = menu("fetch-menu");
    assert!(matches!(action_of(&m, "-p"), Action::Switch(s) if s == "--prune"));
    assert!(matches!(action_of(&m, "-t"), Action::Switch(s) if s == "--tags"));
    assert!(matches!(action_of(&m, "-F"), Action::Switch(s) if s == "--force"));
    assert!(matches!(action_of(&m, "-u"), Action::Switch(s) if s == "--unshallow"));
    for (key, target) in [
        ("p", "fetch-from-pushremote"),
        ("u", "fetch-from-upstream"),
        ("e", "fetch-other"),
        ("a", "fetch-all"),
        ("o", "fetch-branch"),
        ("r", "fetch-refspec"),
        ("m", "fetch-modules"),
        ("C", "branch-config-menu"),
    ] {
        assert!(matches!(action_of(&m, key), Action::Command(c) if c == target), "f {key}");
    }
    assert_eq!(m.keys(), ["-p", "-t", "-u", "-F", "p", "u", "e", "a", "o", "r", "m", "C"]);
    let levels: Vec<u8> = m.groups.iter().flatten().map(|e| e.level).collect();
    assert_eq!(levels, [1, 1, 7, 1, 1, 1, 1, 1, 1, 1, 1, 1]);

    let fm = menu("fetch-modules-menu");
    assert!(matches!(action_of(&fm, "-v"), Action::Switch(s) if s == "--verbose"));
    assert!(matches!(action_of(&fm, "-j"), Action::Option { arg, .. } if arg == "--jobs="));
    assert!(matches!(action_of(&fm, "m"), Action::Command(c) if c == "fetch-modules-run"));
    assert_eq!(xagit_lib::ui::transient::prefix_value("fetch-modules-menu"), ["--verbose", "--jobs=4"]);
}

/// magit-pull.el:42 magit-pull prefix
#[test]
fn pull_popup_layout() {
    let m = menu("pull-menu");
    assert!(matches!(action_of(&m, "-f"), Action::Switch(s) if s == "--ff-only"));
    assert!(matches!(action_of(&m, "-r"),
        Action::Option { arg, reader: xagit_lib::ui::transient::Reader::Pick(c) } if arg == "--rebase=" && *c == ["true", "merges", "interactive", "false"]));
    assert!(matches!(action_of(&m, "-A"), Action::Switch(s) if s == "--autostash"));
    for (key, target) in [
        ("p", "pull-from-pushremote"),
        ("u", "pull-from-upstream"),
        ("e", "pull-branch"),
        ("U", "pull-into-upstream"),
        ("r", "branch.<branch>.rebase"),
        ("C", "branch-config-menu"),
    ] {
        assert!(matches!(action_of(&m, key), Action::Command(c) if c == target), "F {key}");
    }
    assert_eq!(m.keys(), ["-f", "-r", "-A", "-F", "p", "u", "e", "U", "r", "C"]);
    let levels: Vec<u8> = m.groups.iter().flatten().map(|e| e.level).collect();
    assert_eq!(levels, [1, 1, 7, 1, 1, 1, 1, 7, 1, 1]);
}

/// magit-push.el magit-push prefix
#[test]
fn push_popup_layout() {
    let m = menu("push-menu");
    assert!(matches!(action_of(&m, "-f"), Action::Switch(s) if s == "--force-with-lease"));
    assert!(matches!(action_of(&m, "-F"), Action::Switch(s) if s == "--force"));
    assert!(matches!(action_of(&m, "-n"), Action::Switch(s) if s == "--dry-run"));
    assert!(matches!(action_of(&m, "-u"), Action::Switch(s) if s == "--set-upstream"));
    assert!(matches!(action_of(&m, "-h"), Action::Switch(s) if s == "--no-verify"));
    assert!(matches!(action_of(&m, "-T"), Action::Switch(s) if s == "--tags"));
    assert!(matches!(action_of(&m, "-t"), Action::Switch(s) if s == "--follow-tags"));
    assert!(matches!(action_of(&m, "-o"),
        Action::Option { arg, reader: xagit_lib::ui::transient::Reader::Multi(c) } if arg == "--push-option=" && *c == ["skip-ci", "ci.skip"]));
    assert!(matches!(action_of(&m, "p"), Action::Command(c) if c == "push-current-to-pushremote"));
    assert!(matches!(action_of(&m, "u"), Action::Command(c) if c == "push-current-to-upstream"));
    assert!(matches!(action_of(&m, "e"), Action::Command(c) if c == "push-current"));
    assert!(matches!(action_of(&m, "o"), Action::Command(c) if c == "push-other"));
    assert!(matches!(action_of(&m, "n"), Action::Command(c) if c == "push-notes-ref"));
    assert_eq!(m.keys(), ["-f", "-F", "-h", "-n", "-u", "-T", "-t", "-o", "p", "u", "e", "o", "r", "m", "T", "t", "n", "C"]);
    assert!(matches!(action_of(&m, "r"), Action::Command(c) if c == "push-refspecs"));
    assert!(matches!(action_of(&m, "m"), Action::Command(c) if c == "push-matching"));
    // magit-push-tag is "T"; "t" is magit-push-tags (all tags)
    assert!(matches!(action_of(&m, "T"), Action::Command(c) if c == "push-tag"));
    assert!(matches!(action_of(&m, "t"), Action::Command(c) if c == "push-tags"));
    assert!(matches!(action_of(&m, "C"), Action::Command(c) if c == "branch-config-menu"));
    let level = |k: &str| m.groups.iter().flatten().find(|e| e.key == k).unwrap().level;
    assert_eq!((level("-u"), level("-o"), level("n")), (5, 5, 6));
}

/// magit-merge.el:37 magit-merge prefix
#[test]
fn merge_popup_layout() {
    let m = menu("merge-menu");
    let level = |k: &str| m.groups.iter().flatten().find(|e| e.key == k).unwrap().level;
    assert_eq!((level("-b"), level("-w"), level("-s")), (5, 5, 1));
    assert!(matches!(action_of(&m, "-X"), Action::Option { arg, .. } if arg == "--strategy-option="));
    assert!(matches!(action_of(&m, "-A"), Action::Option { arg, .. } if arg == "-Xdiff-algorithm="));
    assert_eq!((level("-X"), level("-A")), (5, 5));
    // magit:--gpg-sign (level 5) and magit:--signoff (level 6)
    for name in ["commit-menu", "merge-menu", "rebase-menu", "cherry-pick-menu", "revert-menu", "patches-menu"] {
        let m = menu(name);
        let e = |k: &str| m.groups.iter().flatten().find(|e| e.key == k).cloned().unwrap_or_else(|| panic!("{name} {k}"));
        assert!(matches!(&e("-S").action, Action::Option { arg, .. } if arg == "--gpg-sign="), "{name}");
        assert!(matches!(&e("+s").action, Action::Switch(s) if s == "--signoff"), "{name}");
        assert_eq!((e("-S").level, e("+s").level), (5, 6), "{name}");
    }
    let c = menu("commit-menu");
    assert!(matches!(action_of(&c, "-D"), Action::Option { arg, .. } if arg == "--date="));
    assert!(matches!(action_of(&m, "-f"), Action::Switch(s) if s == "--ff-only"));
    assert!(matches!(action_of(&m, "-n"), Action::Switch(s) if s == "--no-ff"));
    assert!(matches!(action_of(&m, "m"), Action::Command(c) if c == "merge-plain"));
    assert!(matches!(action_of(&m, "e"), Action::Command(c) if c == "merge-editmsg"));
    assert!(matches!(action_of(&m, "n"), Action::Command(c) if c == "merge-nocommit"));
    assert!(matches!(action_of(&m, "p"), Action::Command(c) if c == "merge-preview"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "merge-squash"));
    assert!(matches!(action_of(&m, "a"), Action::Command(c) if c == "merge-absorb"));
    assert!(matches!(action_of(&m, "d"), Action::Command(c) if c == "merge-dissolve"));

    // :if magit-merge-in-progress-p — "a" aborts only while merging
    let p = menu("merge-menu@progress");
    assert!(matches!(action_of(&p, "m"), Action::Command(c) if c == "merge-commit"));
    assert!(matches!(action_of(&p, "a"), Action::Command(c) if c == "merge-abort"));
    assert!(find(&p, "s").is_none());
    assert!(find(&p, "d").is_none());
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
    assert!(matches!(action_of(&m, "-k"), Action::Switch(s) if s == "--keep-empty"));
    assert!(matches!(action_of(&m, "-r"),
        Action::Option { arg, reader: xagit_lib::ui::transient::Reader::Choices(c) } if arg == "--rebase-merges=" && *c == ["no-rebase-cousins", "rebase-cousins"]));
    assert!(matches!(action_of(&m, "-u"), Action::Switch(s) if s == "--update-refs"));
    assert!(matches!(action_of(&m, "-f"), Action::Switch(s) if s == "--force-rebase"));
    assert!(matches!(action_of(&m, "-x"), Action::Option { arg, .. } if arg == "--exec="));
    assert!(matches!(action_of(&m, "=X"), Action::Option { arg, .. } if arg == "-Xdiff-algorithm="));
    assert!(matches!(action_of(&m, "-S"), Action::Option { arg, .. } if arg == "--gpg-sign="));
    assert!(matches!(action_of(&m, "+s"), Action::Switch(s) if s == "--signoff"));
    assert!(matches!(action_of(&m, "p"), Action::Command(c) if c == "rebase-onto-pushremote"));
    assert!(matches!(action_of(&m, "u"), Action::Command(c) if c == "rebase-onto-upstream"));
    assert!(matches!(action_of(&m, "e"), Action::Command(c) if c == "rebase-branch"));
    let level = |k: &str| m.groups.iter().flatten().find(|e| e.key == k).unwrap().level;
    assert_eq!(
        ["-s", "-X", "=X", "-f", "-x", "-S", "+s", "-r"].map(level),
        [7, 7, 7, 7, 7, 5, 6, 1]
    );
    assert!(matches!(action_of(&m, "f"), Action::Command(c) if c == "rebase-autosquash"));
    assert!(matches!(action_of(&m, "t"), Action::Command(c) if c == "reshelve-since"));
    assert_eq!(m.groups.iter().flatten().find(|e| e.key == "t").unwrap().level, 6);
    assert!(matches!(action_of(&m, "i"), Action::Command(c) if c == "rebase-interactive"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "rebase-subset"));
    assert!(matches!(action_of(&m, "m"), Action::Command(c) if c == "rebase-edit-commit"));
    assert!(matches!(action_of(&m, "w"), Action::Command(c) if c == "rebase-reword-commit"));
    assert!(matches!(action_of(&m, "k"), Action::Command(c) if c == "rebase-remove-commit"));
    // continue/abort only while rebasing (:if magit-rebase-in-progress-p)
    for k in ["r", "a"] {
        assert!(find(&m, k).is_none(), "{k} must not be in the idle rebase menu");
    }

    let p = menu("rebase-menu@progress");
    assert!(matches!(action_of(&p, "r"), Action::Command(c) if c == "rebase-continue"));
    assert!(matches!(action_of(&p, "s"), Action::Command(c) if c == "rebase-skip"));
    assert!(matches!(action_of(&p, "e"), Action::Command(c) if c == "rebase-edit-todo"));
    assert!(matches!(action_of(&p, "a"), Action::Command(c) if c == "rebase-abort"));
    assert!(find(&p, "u").is_none());
    assert!(find(&p, "i").is_none());
}

/// magit-reset.el:34 magit-reset prefix
#[test]
fn reset_popup_layout() {
    let m = menu("reset-menu");
    assert_eq!(m.keys(), ["b", "f", "m", "s", "h", "k", "i", "w"]);
    assert!(matches!(action_of(&m, "b"), Action::Command(c) if c == "branch-reset"));
    assert!(matches!(action_of(&m, "f"), Action::Command(c) if c == "file-checkout"));
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
    assert!(matches!(action_of(&m, "r"), Action::Command(c) if c == "tag-release"));
    assert!(matches!(action_of(&m, "p"), Action::Command(c) if c == "tag-prune"));
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
    assert!(matches!(action_of(&m, "B"), Action::Command(c) if c == "stash-branch-here"));
    assert!(matches!(action_of(&m, "f"), Action::Command(c) if c == "stash-format-patch"));
    assert!(matches!(action_of(&m, "P"), Action::Command(c) if c == "stash-push-menu"));
    let level = |k: &str| m.groups.iter().flatten().find(|e| e.key == k).unwrap().level;
    assert_eq!(level("P"), 5);
    // magit-stash-push
    let m = menu("stash-push-menu");
    assert!(matches!(action_of(&m, "--"), Action::Option { arg, .. } if arg == "-- "));
    for (key, arg) in [("-u", "--include-untracked"), ("-a", "--all"), ("-k", "--keep-index"), ("-K", "--no-keep-index")] {
        assert!(matches!(action_of(&m, key), Action::Switch(s) if s == arg), "{key}");
    }
    assert!(matches!(action_of(&m, "P"), Action::Command(c) if c == "stash-push"));
}

/// magit-gitignore.el magit-gitignore prefix
#[test]
fn gitignore_popup_layout() {
    let m = menu("gitignore-menu");
    assert!(matches!(action_of(&m, "t"), Action::Command(c) if c == "ignore"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "ignore-subdir"));
    assert!(matches!(action_of(&m, "p"), Action::Command(c) if c == "ignore-gitdir"));
    assert!(matches!(action_of(&m, "g"), Action::Command(c) if c == "ignore-system"));
    for (key, target) in [("w", "skip-worktree"), ("W", "no-skip-worktree"), ("u", "assume-unchanged"), ("U", "no-assume-unchanged")] {
        assert!(matches!(action_of(&m, key), Action::Command(c) if c == target), "{key}");
        assert_eq!(m.groups.iter().flatten().find(|e| e.key == key).unwrap().level, 7);
    }
}

/// magit-diff.el magit-diff prefix
#[test]
fn diff_popup_layout() {
    let m = menu("diff-menu");
    assert!(matches!(action_of(&m, "-b"), Action::Switch(s) if s == "--ignore-space-change"));
    assert!(matches!(action_of(&m, "-w"), Action::Switch(s) if s == "--ignore-all-space"));
    assert!(matches!(action_of(&m, "-W"), Action::Switch(s) if s == "--function-context"));
    assert!(matches!(action_of(&m, "d"), Action::Command(c) if c == "diff-dwim"));
    assert!(matches!(action_of(&m, "r"), Action::Command(c) if c == "diff-range"));
    assert!(matches!(action_of(&m, "u"), Action::Command(c) if c == "diff-unstaged"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "diff-staged"));
    assert!(matches!(action_of(&m, "w"), Action::Command(c) if c == "diff-worktree"));
    assert!(matches!(action_of(&m, "c"), Action::Command(c) if c == "show-commit"));
    assert!(matches!(action_of(&m, "t"), Action::Command(c) if c == "stash-show"));

    // magit-diff-refresh ("D"): same switches, "g" re-runs the current diff
    let r = menu("diff-refresh-menu");
    assert!(matches!(action_of(&r, "-w"), Action::Switch(s) if s == "--ignore-all-space"));
    assert!(matches!(action_of(&r, "g"), Action::Command(c) if c == "diff-refresh"));
}

/// magit-log.el magit-log prefix
#[test]
fn log_popup_layout() {
    let m = menu("log-menu");
    assert!(matches!(action_of(&m, "-g"), Action::Switch(s) if s == "--graph"));
    assert!(matches!(action_of(&m, "-d"), Action::Switch(s) if s == "--decorate"));
    assert!(matches!(action_of(&m, "-D"), Action::Switch(s) if s == "--simplify-by-decoration"));
    assert!(matches!(action_of(&m, "-r"), Action::Switch(s) if s == "--reverse"));
    assert!(matches!(action_of(&m, "l"), Action::Command(c) if c == "log-current"));
    assert!(matches!(action_of(&m, "o"), Action::Command(c) if c == "log-other"));
    assert!(matches!(action_of(&m, "L"), Action::Command(c) if c == "log-branches"));
    assert!(matches!(action_of(&m, "b"), Action::Command(c) if c == "log-all-branches"));
    assert!(matches!(action_of(&m, "a"), Action::Command(c) if c == "log-all"));
    assert!(matches!(action_of(&m, "r"), Action::Command(c) if c == "reflog-current"));
    assert!(matches!(action_of(&m, "O"), Action::Command(c) if c == "reflog-other"));
    assert!(matches!(action_of(&m, "H"), Action::Command(c) if c == "reflog-head"));
    assert!(matches!(action_of(&m, "s"), Action::Submenu(c) if c == "shortlog-menu"));
    // options and their values in the popup
    assert!(matches!(action_of(&m, "-n"), Action::Option { arg, .. } if arg == "-n"));
    assert!(matches!(action_of(&m, "-A"), Action::Option { arg, .. } if arg == "--author="));
    assert!(matches!(action_of(&m, "-o"), Action::Switches(a) if a[0] == "--topo-order"));
    let active = std::collections::BTreeMap::from([("-n256".to_string(), true), ("--graph".to_string(), true)]);
    let shown = m.render(&active);
    assert!(shown.contains("-n Limit number of commits (-n256)"), "{shown}");
    assert!(shown.contains("-g Show graph (--graph)"), "{shown}");
    assert!(shown.contains("-A Limit to author ") || shown.contains("-A Limit to author\n"), "{shown}");

    // magit-shortlog ("l s")
    let s = menu("shortlog-menu");
    assert_eq!(s.keys(), ["-n", "-s", "-e", "s", "r"]);
    assert!(matches!(action_of(&s, "s"), Action::Command(c) if c == "shortlog-since"));
    assert!(matches!(action_of(&s, "r"), Action::Command(c) if c == "shortlog-range"));

    // magit-log-refresh ("L")
    let r = menu("log-refresh-menu");
    assert!(matches!(action_of(&r, "-g"), Action::Switch(s) if s == "--graph"));
    assert!(matches!(action_of(&r, "g"), Action::Command(c) if c == "log-refresh"));
}

/// magit-run prefix ("!"), magit.el:418-431
#[test]
fn run_popup_layout() {
    let m = menu("run-menu");
    assert_eq!(m.keys(), ["!", "p", "s", "S", "k", "a", "b", "g", "m"]);
    for (key, target) in [
        ("!", "git-command-topdir"),
        ("p", "git-command"),
        ("s", "shell-command-topdir"),
        ("S", "shell-command"),
        ("k", "run-gitk"),
        ("a", "run-gitk-all"),
        ("b", "run-gitk-branches"),
        ("g", "run-git-gui"),
        ("m", "git-mergetool"),
    ] {
        assert!(matches!(action_of(&m, key), Action::Command(c) if c == target), "! {key}");
    }
}

/// magit-git-mergetool transient (magit-extras.el:46-70)
#[test]
fn mergetool_popup_layout() {
    let m = menu("mergetool-menu");
    assert_eq!(m.keys(), ["-t", "=t", "=T", "-r", "-b", "-k", "-w", " m"]);
    assert!(matches!(action_of(&m, "-t"), Action::Option { arg, reader: Reader::Pick(_) } if arg == "--tool="));
    for (key, var) in [
        ("=t", "merge.guitool"),
        ("=T", "merge.tool"),
        ("-r", "mergetool.hideResolved"),
        ("-b", "mergetool.keepBackup"),
        ("-k", "mergetool.keepTemporaries"),
        ("-w", "mergetool.writeToTemp"),
        (" m", "git-mergetool-invoke"),
    ] {
        assert!(matches!(action_of(&m, key), Action::Command(c) if c == var), "E t {key}");
    }
    // " m" is SPC then m (kbd), not a single key
    assert!(matches!(m.lookup(&xagit_lib::ui::transient::key_seq(" ")), xagit_lib::ui::transient::Lookup::Prefix));
    assert!(matches!(action_of(&m, " m"), Action::Command(c) if c == "git-mergetool-invoke"));
}

/// magit-sequence.el:140 magit-cherry-pick prefix
#[test]
fn cherry_pick_popup_layout() {
    let m = menu("cherry-pick-menu");
    assert!(matches!(action_of(&m, "-x"), Action::Switch(s) if s == "-x"));
    assert!(matches!(action_of(&m, "-e"), Action::Switch(s) if s == "--edit"));
    assert!(matches!(action_of(&m, "A"), Action::Command(c) if c == "cherry-pick"));
    assert!(matches!(action_of(&m, "a"), Action::Command(c) if c == "cherry-apply"));
    assert!(matches!(action_of(&m, "h"), Action::Command(c) if c == "cherry-harvest"));
    assert!(matches!(action_of(&m, "m"), Action::Command(c) if c == "merge-squash"));
    assert!(matches!(action_of(&m, "d"), Action::Command(c) if c == "cherry-donate"));
    assert!(matches!(action_of(&m, "n"), Action::Command(c) if c == "cherry-spinout"));
    assert!(matches!(action_of(&m, "s"), Action::Command(c) if c == "cherry-spinoff"));

    // :if magit-sequencer-in-progress-p
    let p = menu("cherry-pick-menu@progress");
    assert!(matches!(action_of(&p, "A"), Action::Command(c) if c == "sequencer-continue"));
    assert!(matches!(action_of(&p, "s"), Action::Command(c) if c == "sequencer-skip"));
    assert!(matches!(action_of(&p, "a"), Action::Command(c) if c == "sequencer-abort"));
}

/// magit-sequence.el magit-revert prefix ("V")
#[test]
fn revert_popup_layout() {
    let m = menu("revert-menu");
    assert!(matches!(action_of(&m, "-e"), Action::Switch(s) if s == "--edit"));
    assert!(matches!(action_of(&m, "-E"), Action::Switch(s) if s == "--no-edit"));
    assert!(matches!(action_of(&m, "V"), Action::Command(c) if c == "revert"));
    assert!(matches!(action_of(&m, "v"), Action::Command(c) if c == "revert-no-commit"));

    let p = menu("revert-menu@progress");
    assert!(matches!(action_of(&p, "V"), Action::Command(c) if c == "sequencer-continue"));
    assert!(matches!(action_of(&p, "s"), Action::Command(c) if c == "sequencer-skip"));
    assert!(matches!(action_of(&p, "a"), Action::Command(c) if c == "sequencer-abort"));
    assert!(find(&p, "v").is_none());
}

/// Menus with `:if` suffixes switch layout while the operation is in progress.
#[test]
fn in_progress_menus_are_selected_by_repo_state() {
    let idle = RepoState::default();
    for name in ["merge-menu", "rebase-menu", "cherry-pick-menu", "revert-menu", "patches-menu", "bisect-menu"] {
        assert_eq!(resolve_menu(name, idle), name);
    }
    let st = |f: fn(&mut RepoState)| {
        let mut s = RepoState::default();
        f(&mut s);
        s
    };
    assert_eq!(resolve_menu("merge-menu", st(|s| s.merge = true)), "merge-menu@progress");
    assert_eq!(resolve_menu("rebase-menu", st(|s| s.rebase = true)), "rebase-menu@progress");
    assert_eq!(resolve_menu("cherry-pick-menu", st(|s| s.sequencer = true)), "cherry-pick-menu@progress");
    assert_eq!(resolve_menu("revert-menu", st(|s| s.sequencer = true)), "revert-menu@progress");
    assert_eq!(resolve_menu("patches-menu", st(|s| s.am = true)), "patches-menu@progress");
    assert_eq!(resolve_menu("bisect-menu", st(|s| s.bisect = true)), "bisect-menu@progress");
    assert_eq!(resolve_menu("notes-menu", st(|s| s.notes_merging = true)), "notes-menu@progress");
    // a state only affects its own menu
    assert_eq!(resolve_menu("rebase-menu", st(|s| s.merge = true)), "rebase-menu");
    assert_eq!(resolve_menu("commit-menu", st(|s| s.merge = true)), "commit-menu");

    // every variant resolves to a registered menu
    let reg = registry();
    for name in reg.keys().filter(|n| n.ends_with("@progress")) {
        let base = name.trim_end_matches("@progress");
        assert!(reg.contains_key(base), "{name} has no idle menu");
    }
}

/// Every command that names a menu must point at a registered one.
#[test]
fn menu_targets_exist() {
    let reg = registry();
    for (name, m) in &reg {
        for e in m.groups.iter().flatten() {
            if let Action::Command(c) | Action::Submenu(c) = &e.action {
                if c.ends_with("-menu") {
                    assert!(reg.contains_key(c), "{name} {} -> missing {c}", e.key);
                }
            }
        }
    }
}

/// Key translation mirrors Emacs notation.
#[test]
fn key_labels() {
    use xagit_lib::ui::keys::key_from_label;
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
        ("D", "diff-refresh-menu"),
    ] {
        assert!(matches!(action_of(&d, key), Action::Command(c) if c == target), "{key}");
    }

    // magit-bisect: idle menu only starts; marking needs a running bisect
    let b = menu("bisect-menu");
    assert!(matches!(action_of(&b, "B"), Action::Command(c) if c == "bisect-start"));
    assert!(matches!(action_of(&b, "s"), Action::Command(c) if c == "bisect-run"));
    assert!(find(&b, "g").is_none());
    assert_eq!(b.keys(), ["-n", "-p", "=o", "=n", "B", "s"]);
    assert!(matches!(action_of(&b, "-p"), Action::Switch(a) if a == "--first-parent"));
    assert!(matches!(action_of(&b, "=o"), Action::Option { arg, .. } if arg == "--term-old="));
    let b = menu("bisect-menu@progress");
    assert_eq!(b.keys(), ["B", "g", "m", "k", "r", "s"]);
    assert!(matches!(action_of(&b, "B"), Action::Command(c) if c == "bisect-bad"));
    assert!(matches!(action_of(&b, "g"), Action::Command(c) if c == "bisect-good"));
    assert!(matches!(action_of(&b, "k"), Action::Command(c) if c == "bisect-skip"));
    assert!(matches!(action_of(&b, "r"), Action::Command(c) if c == "bisect-reset"));
    assert!(matches!(action_of(&b, "s"), Action::Command(c) if c == "bisect-run"));

    let w = menu("worktree-menu");
    assert!(matches!(action_of(&w, "k"), Action::Command(c) if c == "worktree-delete"));

    // magit-submodule: "r" registers (git submodule init), "k" removes
    let s = menu("submodule-menu");
    for (key, target) in [
        ("a", "submodule-add"),
        ("r", "submodule-register"),
        ("p", "submodule-populate"),
        ("u", "submodule-update"),
        ("s", "submodule-sync"),
        ("d", "submodule-unpopulate"),
        ("k", "submodule-remove"),
        ("l", "submodule-list"),
        ("f", "fetch-modules"),
    ] {
        assert!(matches!(action_of(&s, key), Action::Command(c) if c == target), "o {key}");
    }
    for (key, flag) in [("-f", "--force"), ("-r", "--recursive"), ("-N", "--no-fetch"), ("-U", "--remote")] {
        assert!(matches!(action_of(&s, key), Action::Switch(x) if x == flag), "o {key}");
    }

    let n = menu("notes-menu");
    assert!(matches!(action_of(&n, "T"), Action::Command(c) if c == "notes-edit"));
    assert_eq!(n.keys(), ["c", "d", "C", "D", "-n", "-r", "-s", "T", "r", "m", "p"]);
    assert!(matches!(action_of(&n, "-r"), Action::Option { arg, reader: Reader::NotesRef } if arg == "--ref="));
    assert!(matches!(action_of(&n, "C"), Action::Command(c) if c == "global-core.notesRef"));
    let n = menu("notes-menu@progress");
    assert_eq!(n.keys(), ["d", "C", "D", "c", "a"]);
    assert!(matches!(action_of(&n, "c"), Action::Command(c) if c == "notes-merge-commit"));

    // magit-ediff (magit-ediff.el:117-131): "E" is dwim (shown as text
    // diffs in xagit), the stage/resolve suffixes use git commands
    let e = menu("ediff-menu");
    assert_eq!(e.keys(), ["E", "s", "m", "M", "t", "u", "i", "w", "c", "r", "z"]);
    for (key, target) in [
        ("E", "ediff-dwim"),
        ("s", "ediff-stage"),
        ("m", "ediff-resolve-rest"),
        ("M", "ediff-resolve-all"),
        ("t", "ediff-mergetool"),
        ("u", "ediff-unstaged"),
        ("i", "ediff-staged"),
        ("w", "ediff-worktree"),
        ("c", "ediff-commit"),
        ("r", "ediff-range"),
        ("z", "ediff-stash"),
    ] {
        assert!(matches!(action_of(&e, key), Action::Command(c) if c == target), "E {key}");
    }
}

/// No transient entry may remain a placeholder.
#[test]
fn no_unsupported_commands_remain() {
    for (name, m) in registry() {
        for e in m.groups.iter().flatten() {
            assert!(
                !matches!(&e.action, Action::Command(c) if c == "unsupported"),
                "{name}: {} still unsupported",
                e.key
            );
        }
    }
}

/// The last batch of Magit extras: subtree, patches, branch config, reflog, etc.
#[test]
fn remaining_extras_popup_layout() {
    let reg = registry();
    for name in ["subtree-menu", "patches-menu", "branch-config-menu"] {
        assert!(reg.contains_key(name), "{name} missing");
    }
    let d = menu("dispatch");
    for (key, target) in [
        ("H", "section-info"),
        ("J", "display-buffer"),
        ("L", "log-refresh-menu"),
        ("O", "subtree-menu"),
        ("w", "patches-menu"),
        ("W", "patch-menu"),
        ("Y", "cherries"),
    ] {
        assert!(matches!(action_of(&d, key), Action::Command(c) if c == target), "{key}");
    }
    let b = menu("branch-menu");
    assert!(matches!(action_of(&b, "C"), Action::Command(c) if c == "branch-config-menu"));
    let l = menu("log-menu");
    assert!(matches!(action_of(&l, "r"), Action::Command(c) if c == "reflog-current"));
    // magit-subtree ("O"), -import and -export
    let s = menu("subtree-menu");
    assert_eq!(s.keys(), ["i", "e"]);
    assert!(matches!(action_of(&s, "i"), Action::Command(c) if c == "subtree-import-menu"));
    assert!(matches!(action_of(&s, "e"), Action::Command(c) if c == "subtree-export-menu"));
    let s = menu("subtree-import-menu");
    assert_eq!(s.keys(), ["-P", "-m", "-s", "a", "c", "m", "f"]);
    assert!(matches!(action_of(&s, "-P"), Action::Option { arg, reader: Reader::SubtreePrefix } if arg == "--prefix="));
    assert!(matches!(action_of(&s, "c"), Action::Command(c) if c == "subtree-add-commit"));
    assert!(matches!(action_of(&s, "f"), Action::Command(c) if c == "subtree-pull"));
    let s = menu("subtree-export-menu");
    assert_eq!(s.keys(), ["-P", "-a", "-b", "-o", "-i", "-j", "p", "s"]);
    assert!(matches!(action_of(&s, "-o"), Action::Option { arg, reader: Reader::Revision } if arg == "--onto="));
    assert!(matches!(action_of(&s, "-j"), Action::Switch(a) if a == "--rejoin"));

    // magit-am ("w")
    let p = menu("patches-menu");
    assert!(matches!(action_of(&p, "-3"), Action::Switch(s) if s == "--3way"));
    assert!(matches!(action_of(&p, "m"), Action::Command(c) if c == "patches-apply-maildir"));
    assert!(matches!(action_of(&p, "w"), Action::Command(c) if c == "patches-apply"));
    assert!(matches!(action_of(&p, "a"), Action::Command(c) if c == "patch-apply-menu"));
    assert!(matches!(action_of(&p, "-p"), Action::Option { arg, reader: Reader::OptionalNumber } if arg == "-p"));
    let p = menu("patches-menu@progress");
    assert_eq!(p.keys(), ["w", "s", "a"]);
    assert!(matches!(action_of(&p, "w"), Action::Command(c) if c == "patches-continue"));
    assert!(matches!(action_of(&p, "s"), Action::Command(c) if c == "patches-skip"));
    assert!(matches!(action_of(&p, "a"), Action::Command(c) if c == "patches-abort"));

    // magit-patch ("W")
    let p = menu("patch-menu");
    assert_eq!(p.keys(), ["c", "w", "a", "s", "r"]);
    assert!(matches!(action_of(&p, "c"), Action::Command(c) if c == "patch-create-menu"));
    assert!(matches!(action_of(&p, "w"), Action::Command(c) if c == "patches-menu"));
    assert!(matches!(action_of(&p, "a"), Action::Command(c) if c == "patch-apply-menu"));
    assert!(matches!(action_of(&p, "s"), Action::Command(c) if c == "patch-save"));
    assert!(matches!(action_of(&p, "r"), Action::Command(c) if c == "request-pull"));
    // magit-patch-create
    let p = menu("patch-create-menu");
    assert_eq!(
        p.keys(),
        [
            "C-m C-r", "C-m s", "C-m C-f", "C-m C-t", "C-m C-c", "C-m b", "C-m v", "C-m d i", "C-m d r", "C-m p", "C-m r",
            "C-m l", "C-m D", "C-m n", "C-m o", "-U", "-M", "-C", "-A", "--", "-b", "-w", "c"
        ]
    );
    assert!(matches!(action_of(&p, "C-m b"), Action::Option { arg, reader: Reader::PatchBase } if arg == "--base="));
    assert!(matches!(action_of(&p, "C-m l"), Action::Switch(a) if a == "--cover-letter"));
    assert!(matches!(action_of(&p, "--"), Action::Option { reader: Reader::TrackedFiles, .. }));
    assert!(matches!(action_of(&p, "c"), Action::Command(c) if c == "patch-create"));
    // magit-patch-apply
    let p = menu("patch-apply-menu");
    assert_eq!(p.keys(), ["-i", "-c", "-3", "a"]);
    assert!(matches!(action_of(&p, "a"), Action::Command(c) if c == "patch-apply"));

    // magit-branch-configure (magit-branch.el)
    let c = menu("branch-config-menu");
    for (key, target) in [
        ("d", "branch.<branch>.description"),
        ("u", "branch.<branch>.merge/remote"),
        ("r", "branch.<branch>.rebase"),
        ("p", "branch.<branch>.pushRemote"),
        ("R", "pull.rebase"),
        ("P", "remote.pushDefault"),
        ("B", "update-default-branch"),
        ("a m", "branch.autoSetupMerge"),
        ("a r", "branch.autoSetupRebase"),
    ] {
        assert!(matches!(action_of(&c, key), Action::Command(x) if x == target), "b C {key}");
    }
    assert_eq!(c.keys(), ["d", "u", "r", "p", "R", "P", "B", "a m", "a r"]);

    // magit-remote-configure (magit-remote.el)
    let c = menu("remote-configure-menu");
    for (key, target) in [
        ("u", "remote.<remote>.url"),
        ("U", "remote.<remote>.fetch"),
        ("s", "remote.<remote>.pushurl"),
        ("S", "remote.<remote>.push"),
        ("O", "remote.<remote>.tagopt"),
        ("h", "remote.<remote>.followremotehead"),
    ] {
        assert!(matches!(action_of(&c, key), Action::Command(x) if x == target), "M C {key}");
    }
    assert_eq!(c.keys(), ["u", "U", "s", "S", "O", "h"]);
}

/// Transient keys are key sequences, as in Emacs `kbd`.
#[test]
fn transient_key_sequences() {
    use xagit_lib::ui::keys::key_from_label;
    use xagit_lib::ui::transient::{key_seq, Lookup};
    let k = key_from_label;
    assert_eq!(key_seq("-a"), vec![k("-"), k("a")]);
    assert_eq!(key_seq("a m"), vec![k("a"), k("m")]);
    assert_eq!(key_seq("C-c"), vec![k("C-c")]);
    assert_eq!(key_seq("<tab>"), vec![k("tab")]);
    assert_eq!(key_seq("!"), vec![k("!")]);

    let c = menu("branch-config-menu");
    assert!(matches!(c.lookup(&[k("a")]), Lookup::Prefix));
    assert!(matches!(c.lookup(&[k("a"), k("m")]), Lookup::Found(e) if e.key == "a m"));
    assert!(matches!(c.lookup(&[k("a"), k("z")]), Lookup::None));
    let p = menu("push-menu");
    assert!(matches!(p.lookup(&[k("-")]), Lookup::Prefix));
    assert!(matches!(p.lookup(&[k("-"), k("f")]), Lookup::Found(e) if e.key == "-f"));
    assert!(matches!(p.lookup(&[k("f")]), Lookup::None), "f alone is not -f");

    // like transient: no binding may shadow another or be defined twice
    for (name, m) in registry() {
        let seqs: Vec<(String, Vec<_>)> = m.groups.iter().flatten().map(|e| (e.key.clone(), key_seq(&e.key))).collect();
        for (i, (a, sa)) in seqs.iter().enumerate() {
            for (b, sb) in seqs.iter().skip(i + 1) {
                assert!(sa != sb, "{name}: {a} defined twice");
                assert!(!sb.starts_with(sa) && !sa.starts_with(sb), "{name}: {a} and {b} overlap");
            }
        }
    }
}

/// Typing a switch is two keys ("-" then "f"); "a m" likewise.
#[test]
fn app_reads_multi_key_bindings() {
    use xagit_lib::prelude::*;
    use xagit_lib::ui::app::App;
    let td = TempDir::new().unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(td.path())
        .status()
        .unwrap()
        .success());
    assert!(std::process::Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"])
        .current_dir(td.path())
        .status()
        .unwrap()
        .success());
    let g = Git::discover(td.path()).unwrap();
    let mut app = App::new(g.clone());

    app.press("P").unwrap();
    app.press("-").unwrap();
    assert_eq!(app.current_menu(), Some("push-menu"), "half a key keeps the menu open");
    app.press("f").unwrap();
    assert_eq!(app.active_switches(), vec!["--force-with-lease".to_string()]);
    app.press("-").unwrap();
    app.press("f").unwrap();
    assert!(app.active_switches().is_empty(), "second -f toggles off");
    app.press("-").unwrap();
    app.press("z").unwrap();
    assert_eq!(app.message(), "Unbound suffix: `- z' (Use `C-g' to abort, `?' for help) [nil]");
    assert_eq!(app.current_menu(), Some("push-menu"));
    // after an undefined sequence the next key starts afresh
    app.press("-").unwrap();
    app.press("n").unwrap();
    assert_eq!(app.active_switches(), vec!["--dry-run".to_string()]);
    // -u (--set-upstream) is at level 5
    app.press("-").unwrap();
    app.press("u").unwrap();
    assert_eq!(app.active_switches(), vec!["--dry-run".to_string()]);
    app.press("C-g").unwrap();
    assert_eq!(app.current_menu(), None);

    let local = |k: &str| {
        g.git_string(&["config", "--local", k])
            .ok()
            .filter(|s| !s.is_empty())
    };
    // "b C" reads the branch to configure; the variables are set without
    // leaving the transient
    let branch = g.current_branch().unwrap();
    app.script_answers(&[&branch]);
    app.press("b").unwrap();
    app.press("C").unwrap();
    assert_eq!(app.current_menu(), Some("branch-config-menu"));
    assert_eq!(app.prompts().last().unwrap(), "Configure branch");
    for want in [Some("always"), Some("true"), Some("false"), None] {
        app.press("a").unwrap();
        app.press("m").unwrap();
        assert_eq!(app.current_menu(), Some("branch-config-menu"));
        assert_eq!(local("branch.autoSetupMerge").as_deref(), want);
    }
    app.press("a").unwrap();
    app.press("r").unwrap();
    assert_eq!(local("branch.autoSetupRebase").as_deref(), Some("always"));
    app.press("C-g").unwrap();

    // "b r" (direct configure) cycles branch.<b>.rebase through Magit's choices
    let key = format!("branch.{branch}.rebase");
    app.press("b").unwrap();
    for want in [Some("true"), Some("merges"), Some("interactive"), Some("false"), None] {
        app.press("r").unwrap();
        assert_eq!(app.current_menu(), Some("branch-menu"));
        assert_eq!(local(&key).as_deref(), want);
    }
    app.press("C-g").unwrap();
}

/// magit-status-mode-map: `k` is magit-discard, `j` is magit-status-jump;
/// there is no vi-style movement (move with n / p).
#[test]
fn status_buffer_direct_keys() {
    use xagit_lib::ui::app::{status_binding, Binding};
    use xagit_lib::ui::keys::key_from_label;
    let b = |l: &str| status_binding(&key_from_label(l));
    assert_eq!(b("k"), Some(Binding::Command("discard")));
    assert_eq!(b("K"), Some(Binding::Command("untrack")));
    assert_eq!(b("s"), Some(Binding::Command("stage")));
    assert_eq!(b("u"), Some(Binding::Command("unstage")));
    // magit-section-mode-map
    assert_eq!(b("n"), Some(Binding::SectionForward));
    assert_eq!(b("p"), Some(Binding::SectionBackward));
    assert_eq!(b("M-n"), Some(Binding::SiblingForward));
    assert_eq!(b("M-p"), Some(Binding::SiblingBackward));
    assert_eq!(b("C-<tab>"), Some(Binding::Cycle));
    assert_eq!(b("<backtab>"), Some(Binding::CycleGlobal));
    assert_eq!(b("1"), Some(Binding::ShowLevel(1)));
    assert_eq!(b("4"), Some(Binding::ShowLevel(4)));
    assert_eq!(b("M-2"), Some(Binding::ShowLevelAll(2)));
    assert_eq!(b("C-SPC"), Some(Binding::SetMark));
    assert_eq!(b("C-@"), Some(Binding::SetMark));
    assert_eq!(b("C-g"), Some(Binding::KeyboardQuit));
    // magit-mode-map
    assert_eq!(b("M-<tab>"), Some(Binding::CycleDiffs));
    assert_eq!(b("SPC"), Some(Binding::Command("diff-show-or-scroll-up")));
    assert_eq!(b("DEL"), Some(Binding::Command("diff-show-or-scroll-down")));
    assert_eq!(b("+"), Some(Binding::Command("diff-more-context")));
    assert_eq!(b("-"), Some(Binding::Command("diff-less-context")));
    assert_eq!(b("0"), Some(Binding::Command("diff-default-context")));
    assert_eq!(b("<up>"), Some(Binding::Up));
    assert_eq!(b("<down>"), Some(Binding::Down));
    assert_eq!(b("C-p"), Some(Binding::Up));
    assert_eq!(b("C-n"), Some(Binding::Down));
    assert_eq!(b("j"), Some(Binding::Command("jump")));
    assert_eq!(b("^"), Some(Binding::Command("section-up")));
    assert_eq!(b("<tab>"), Some(Binding::Toggle));
    assert_eq!(b("RET"), Some(Binding::Visit));
    assert_eq!(b("q"), Some(Binding::Bury));
    assert_eq!(b("g"), Some(Binding::Refresh));
    assert_eq!(b("c"), Some(Binding::Menu("commit-menu")));
    assert_eq!(b("C-k"), None);

    // keys whose meaning used to differ from magit-mode-map
    assert_eq!(b("$"), Some(Binding::Command("process-buffer")));
    assert_eq!(b("a"), Some(Binding::Command("apply")));
    assert_eq!(b("v"), Some(Binding::Command("reverse")));
    assert_eq!(b("e"), Some(Binding::Command("ediff-dwim")));
    assert_eq!(b("D"), Some(Binding::Menu("diff-refresh-menu")));
    assert_eq!(b("L"), Some(Binding::Menu("log-refresh-menu")));
    assert_eq!(b("V"), Some(Binding::Menu("revert-menu")));
    assert_eq!(b("%"), Some(Binding::Menu("worktree-menu")));
    assert_eq!(b("Z"), Some(Binding::Menu("worktree-menu")));
    assert_eq!(b("w"), Some(Binding::Menu("patches-menu")));
    assert_eq!(b("W"), Some(Binding::Menu("patch-menu")));
    assert_eq!(b("o"), Some(Binding::Menu("submodule-menu")));

    // no direct key may be shadowed: every target must be a menu or a command
    for l in ["s", "S", "u", "U", "k", "K", "x", "V", "y", "c", "b", "f", "F", "P", "z", "i", "l", "d", "A"] {
        assert!(b(l).is_some(), "{l} unbound");
    }
    // every direct menu key opens a registered menu
    let reg = registry();
    for c in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!$%^:?".chars() {
        if let Some(Binding::Menu(m)) = b(&c.to_string()) {
            assert!(reg.contains_key(m), "{c} -> missing menu {m}");
        }
    }
}

/// Pressing keys in the App opens the layout Magit would show.
#[test]
fn app_opens_in_progress_menu_variants() {
    use xagit_lib::prelude::*;
    use xagit_lib::ui::app::App;
    let td = TempDir::new().unwrap();
    let run = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(td.path())
            .output()
            .unwrap()
            .status
            .success());
    };
    run(&["init", "-q"]);
    run(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"]);
    let g = Git::discover(td.path()).unwrap();
    let mut app = App::new(g.clone());

    for (key, want) in [
        ("m", "merge-menu"),
        ("r", "rebase-menu"),
        ("A", "cherry-pick-menu"),
        ("V", "revert-menu"),
        ("B", "bisect-menu"),
        ("w", "patches-menu"),
        ("D", "diff-refresh-menu"),
        ("L", "log-refresh-menu"),
        ("W", "patch-menu"),
    ] {
        app.press(key).unwrap();
        assert_eq!(app.current_menu(), Some(want), "{key}");
        app.press("C-g").unwrap();
        assert_eq!(app.current_menu(), None);
    }
    // W w opens the am transient
    app.press("W").unwrap();
    app.press("w").unwrap();
    assert_eq!(app.current_menu(), Some("patches-menu"));
    app.press("q").unwrap();

    // simulate in-progress states via the files Magit's predicates check
    std::fs::write(g.gitdir.join("MERGE_HEAD"), g.rev_parse("HEAD").unwrap()).unwrap();
    app.press("m").unwrap();
    assert_eq!(app.current_menu(), Some("merge-menu@progress"));
    app.press("q").unwrap();
    std::fs::remove_file(g.gitdir.join("MERGE_HEAD")).unwrap();

    std::fs::write(g.gitdir.join("BISECT_LOG"), "").unwrap();
    app.press("B").unwrap();
    assert_eq!(app.current_menu(), Some("bisect-menu@progress"));
    app.press("q").unwrap();
    std::fs::remove_file(g.gitdir.join("BISECT_LOG")).unwrap();

    std::fs::create_dir_all(g.gitdir.join("rebase-apply")).unwrap();
    std::fs::write(g.gitdir.join("rebase-apply/applying"), "").unwrap();
    // git am in progress is not a rebase (magit-rebase-in-progress-p)
    app.press("r").unwrap();
    assert_eq!(app.current_menu(), Some("rebase-menu"));
    app.press("q").unwrap();
    app.press("w").unwrap();
    assert_eq!(app.current_menu(), Some("patches-menu@progress"));
    app.press("q").unwrap();
    std::fs::remove_file(g.gitdir.join("rebase-apply/applying")).unwrap();
    // a bare rebase-apply/ directory is neither (magit checks rebase-apply/onto)
    app.press("r").unwrap();
    assert_eq!(app.current_menu(), Some("rebase-menu"));
    app.press("q").unwrap();
    std::fs::write(g.gitdir.join("rebase-apply/onto"), "").unwrap();
    app.press("r").unwrap();
    assert_eq!(app.current_menu(), Some("rebase-menu@progress"));
    app.press("q").unwrap();
    std::fs::remove_dir_all(g.gitdir.join("rebase-apply")).unwrap();
    std::fs::create_dir_all(g.gitdir.join("rebase-merge")).unwrap();
    app.press("r").unwrap();
    assert_eq!(app.current_menu(), Some("rebase-menu@progress"));
    app.press("q").unwrap();
}

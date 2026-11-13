//! ui/theme.rs — faces, palettes and what the terminal can show.
//!
//! Lines are painted with semantic faces (like Magit's magit-hash,
//! magit-branch-local, magit-diff-added …) that a theme maps to colors.
//! The theme never paints the default background: it keeps the user's
//! terminal colors and only tints highlighted sections, diff lines and
//! the bars.
//!
//! What the terminal supports is detected from the environment, the same
//! way on Linux and macOS:
//!
//! - `XAGIT_COLOR` = `truecolor` | `256` | `16` | `never` overrides it;
//!   `NO_COLOR` (no-color.org) and `TERM=dumb` turn colors off;
//! - `COLORTERM=truecolor|24bit`, or a terminal known to support 24-bit
//!   colors (iTerm2, WezTerm, kitty, Ghostty, Alacritty, foot, VS Code,
//!   VTE terminals), give true colors; macOS Terminal.app and `*-256color`
//!   get 256 colors; the Linux console (`TERM=linux`) 16.
//! - `XAGIT_THEME` = `light` | `dark` selects the palette; otherwise
//!   `COLORFGBG`, then on macOS Terminal.app the system appearance (its
//!   default profile follows it), else dark.
//! - `XAGIT_ASCII=1`, the Linux console, and a non-UTF-8 locale on Linux
//!   use ASCII glyphs instead of ▸ ▾ ▌ ─.

use crossterm::style::Color;

/// How many colors the terminal shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Mono,
    Basic,
    Ansi256,
    True,
}

/// The semantic faces of painted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Face {
    Default,
    Dim,
    /// magit-section-heading
    SectionHeading,
    /// the "(N)" after a section heading
    Count,
    /// "Head:", "Author:" …
    Label,
    /// magit-hash
    Hash,
    /// magit-branch-local
    BranchLocal,
    /// magit-branch-current
    BranchCurrent,
    /// magit-branch-remote
    BranchRemote,
    /// magit-tag
    Tag,
    /// "@" (magit-head)
    Head,
    /// magit-bisect-good/bad/skip
    Bisect,
    /// stash@{N}
    Stash,
    /// the refs of a log line (subdivided by name by the painter)
    Refs,
    /// "N>" / "<N" of the refs buffer
    Ahead,
    /// a commit line's graph (colored by column)
    Graph,
    /// the "pick"/"edit" … word of a sequence line, cherry marks
    Keyword,
    /// magit-sequence-pick / -stop / -part / -head / -drop / -done / -onto:
    /// the type words of the rebase / sequencer / am sections
    SequencePick,
    SequenceStop,
    SequencePart,
    SequenceHead,
    SequenceDrop,
    SequenceDone,
    SequenceOnto,
    /// magit-diff-file-heading
    FileHeading,
    FileModified,
    FileNew,
    FileDeleted,
    FileRenamed,
    FileUnmerged,
    /// magit-diff-hunk-heading
    HunkHeading,
    /// the @@ … @@ of a hunk heading
    HunkRange,
    /// magit-diff-added / -removed / -context
    Added,
    Removed,
    Context,
    /// conflict markers (smerge-markers)
    Conflict,
    /// rename / chmod pseudo-hunks
    Meta,
    /// magit-blame-heading
    BlameHeading,
    /// the right margin (author, age)
    Margin,
    /// the bars
    HeaderBar,
    HeaderTitle,
    Badge,
    ModeBadge,
    ModeLine,
    ModeLineDim,
    Message,
    Error,
    HintKey,
    HintText,
    /// the transient window
    PopupRule,
    PopupTitle,
    PopupKey,
    PopupDesc,
    PopupValue,
    PopupInapt,
    PopupGroup,
    /// the minibuffer
    Prompt,
    Completion,
    /// the cursor's line and section (magit-section-highlight), the region
    CursorLine,
    SectionHighlight,
    Region,
    /// the character at point in a built-in text buffer
    Point,
    /// matches of the current buffer search (C-s)
    SearchMatch,
    Gutter,
    Fringe,
}

/// A color in the palette (24-bit; reduced to what the terminal shows).
pub type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl Style {
    const fn fg(c: Rgb) -> Style {
        Style { fg: Some(c), bg: None, bold: false, dim: false, italic: false, underline: false, reverse: false }
    }
    const fn bold(mut self) -> Style {
        self.bold = true;
        self
    }
    const fn on(mut self, c: Rgb) -> Style {
        self.bg = Some(c);
        self
    }
    const fn italic(mut self) -> Style {
        self.italic = true;
        self
    }

    /// This style over UNDER: unset colors come from UNDER (a face's text
    /// on the background of the highlighted line).
    pub fn over(self, under: Style) -> Style {
        Style {
            fg: self.fg.or(under.fg),
            bg: self.bg.or(under.bg),
            bold: self.bold || under.bold,
            dim: self.dim || under.dim,
            italic: self.italic || under.italic,
            underline: self.underline || under.underline,
            reverse: self.reverse || under.reverse,
        }
    }
}

/// The glyphs of the fringe and the bars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyphs {
    pub folded: char,
    pub unfolded: char,
    pub cursor: char,
    pub region: char,
    pub rule: char,
    pub sep: &'static str,
    /// the frames of the busy indicator of the mode line
    pub spinner: &'static [char],
}

const UNICODE: Glyphs = Glyphs {
    folded: '▸',
    unfolded: '▾',
    cursor: '▌',
    region: '┃',
    rule: '─',
    sep: " · ",
    spinner: &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'],
};
const ASCII: Glyphs = Glyphs {
    folded: '>',
    unfolded: 'v',
    cursor: '>',
    region: '|',
    rule: '-',
    sep: " - ",
    spinner: &['|', '/', '-', '\\'],
};

struct Palette {
    accent: Rgb,
    accent_fg: Rgb,
    heading: Rgb,
    dim: Rgb,
    hash: Rgb,
    local: Rgb,
    remote: Rgb,
    tag: Rgb,
    head: Rgb,
    stash: Rgb,
    keyword: Rgb,
    added: Rgb,
    added_bg: Rgb,
    removed: Rgb,
    removed_bg: Rgb,
    modified: Rgb,
    hunk: Rgb,
    hunk_bg: Rgb,
    conflict: Rgb,
    bar_bg: Rgb,
    bar_fg: Rgb,
    mode_bg: Rgb,
    highlight_bg: Rgb,
    cursor_bg: Rgb,
    region_bg: Rgb,
    popup_key: Rgb,
    error: Rgb,
    graph: [Rgb; 6],
}

/// "Ember": warm amber accents on the terminal's own dark background.
const DARK: Palette = Palette {
    accent: (0xF2, 0xA6, 0x5A),
    accent_fg: (0x1A, 0x1B, 0x20),
    heading: (0xB4, 0xBE, 0xFE),
    dim: (0x7C, 0x80, 0x8C),
    hash: (0xB3, 0x9E, 0x72),
    local: (0x8C, 0xC4, 0x8A),
    remote: (0x7F, 0xB4, 0xE0),
    tag: (0xE8, 0xC1, 0x70),
    head: (0xF2, 0x8C, 0x6B),
    stash: (0xC9, 0xA0, 0xDC),
    keyword: (0x81, 0xC8, 0xBE),
    added: (0xA6, 0xD1, 0x89),
    added_bg: (0x1E, 0x2B, 0x20),
    removed: (0xE7, 0x82, 0x84),
    removed_bg: (0x31, 0x1F, 0x21),
    modified: (0xE8, 0xC1, 0x70),
    hunk: (0xA9, 0xB1, 0xD6),
    hunk_bg: (0x26, 0x29, 0x36),
    conflict: (0xF2, 0x8C, 0x6B),
    bar_bg: (0x22, 0x24, 0x2C),
    bar_fg: (0xC8, 0xCC, 0xD6),
    mode_bg: (0x7F, 0xB4, 0xE0),
    highlight_bg: (0x1F, 0x21, 0x28),
    cursor_bg: (0x2C, 0x2F, 0x3A),
    region_bg: (0x3A, 0x3E, 0x5A),
    popup_key: (0xC6, 0xA0, 0xF6),
    error: (0xE7, 0x82, 0x84),
    graph: [
        (0xF2, 0xA6, 0x5A),
        (0x7F, 0xB4, 0xE0),
        (0x8C, 0xC4, 0x8A),
        (0xC6, 0xA0, 0xF6),
        (0xE8, 0xC1, 0x70),
        (0x81, 0xC8, 0xBE),
    ],
};

/// The same hues, darker, for light backgrounds.
const LIGHT: Palette = Palette {
    accent: (0xB8, 0x5C, 0x00),
    accent_fg: (0xFF, 0xFF, 0xFF),
    heading: (0x3F, 0x45, 0xA8),
    dim: (0x80, 0x84, 0x8E),
    hash: (0x8A, 0x6D, 0x2F),
    local: (0x2E, 0x7D, 0x32),
    remote: (0x1F, 0x5F, 0xAD),
    tag: (0x9A, 0x6A, 0x00),
    head: (0xC2, 0x41, 0x0C),
    stash: (0x7B, 0x3F, 0xA0),
    keyword: (0x00, 0x79, 0x6B),
    added: (0x22, 0x6B, 0x2A),
    added_bg: (0xE6, 0xF4, 0xE4),
    removed: (0xB0, 0x24, 0x2A),
    removed_bg: (0xFB, 0xE7, 0xE7),
    modified: (0x9A, 0x6A, 0x00),
    hunk: (0x4A, 0x52, 0x7A),
    hunk_bg: (0xEC, 0xEE, 0xF5),
    conflict: (0xC2, 0x41, 0x0C),
    bar_bg: (0xEE, 0xEC, 0xE6),
    bar_fg: (0x3A, 0x3D, 0x45),
    mode_bg: (0x1F, 0x5F, 0xAD),
    highlight_bg: (0xF5, 0xF3, 0xEE),
    cursor_bg: (0xEA, 0xE6, 0xDC),
    region_bg: (0xD6, 0xDE, 0xF2),
    popup_key: (0x7A, 0x3E, 0xC0),
    error: (0xB0, 0x24, 0x2A),
    graph: [
        (0xB8, 0x5C, 0x00),
        (0x1F, 0x5F, 0xAD),
        (0x2E, 0x7D, 0x32),
        (0x7A, 0x3E, 0xC0),
        (0x9A, 0x6A, 0x00),
        (0x00, 0x7A, 0x70),
    ],
};

/// The faces' styles for one terminal.
#[derive(Debug, Clone)]
pub struct Theme {
    pub depth: Depth,
    pub light: bool,
    pub glyphs: Glyphs,
}

impl Default for Theme {
    fn default() -> Self {
        Theme { depth: Depth::Mono, light: false, glyphs: UNICODE }
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The color depth from TERM, COLORTERM and TERM_PROGRAM (see the module
/// documentation).
pub fn detect_depth(get: &dyn Fn(&str) -> Option<String>) -> Depth {
    if let Some(v) = get("XAGIT_COLOR") {
        match v.to_ascii_lowercase().as_str() {
            "never" | "none" | "no" | "off" | "0" | "mono" => return Depth::Mono,
            "16" | "basic" | "ansi" => return Depth::Basic,
            "256" => return Depth::Ansi256,
            "truecolor" | "24bit" | "true" => return Depth::True,
            _ => {}
        }
    }
    if get("NO_COLOR").is_some() {
        return Depth::Mono;
    }
    let term = get("TERM").unwrap_or_default().to_ascii_lowercase();
    if term == "dumb" {
        return Depth::Mono;
    }
    if matches!(get("COLORTERM").as_deref().map(str::to_ascii_lowercase).as_deref(), Some("truecolor" | "24bit")) {
        return Depth::True;
    }
    let program = get("TERM_PROGRAM").unwrap_or_default();
    if matches!(program.as_str(), "iTerm.app" | "WezTerm" | "ghostty" | "vscode" | "Hyper" | "Tabby" | "rio") {
        return Depth::True;
    }
    if ["kitty", "alacritty", "foot", "wezterm", "ghostty", "-direct"].iter().any(|t| term.contains(t)) {
        return Depth::True;
    }
    if get("VTE_VERSION").and_then(|v| v.parse::<u32>().ok()).is_some_and(|v| v >= 3600) {
        return Depth::True;
    }
    if term.contains("256color") || program == "Apple_Terminal" {
        return Depth::Ansi256;
    }
    if term == "linux" || term.starts_with("vt") || term == "ansi" || term == "cons25" {
        return Depth::Basic;
    }
    if term.starts_with("xterm") || term.starts_with("screen") || term.starts_with("tmux") || term.starts_with("rxvt") {
        return Depth::Ansi256;
    }
    Depth::Basic
}

/// Whether the background is light (see the module documentation).
pub fn detect_light(get: &dyn Fn(&str) -> Option<String>, macos_dark: &dyn Fn() -> Option<bool>) -> bool {
    if let Some(v) = get("XAGIT_THEME") {
        match v.to_ascii_lowercase().as_str() {
            "light" => return true,
            "dark" => return false,
            _ => {}
        }
    }
    // rxvt, Konsole and others: "FG;BG" or "FG;default;BG"
    if let Some(bg) = get("COLORFGBG").and_then(|v| v.rsplit(';').next().and_then(|b| b.parse::<u8>().ok())) {
        return bg == 7 || bg >= 9;
    }
    if get("TERM_PROGRAM").as_deref() == Some("Apple_Terminal") {
        if let Some(dark) = macos_dark() {
            return !dark;
        }
    }
    false
}

/// Whether to use ASCII glyphs (see the module documentation).
pub fn detect_ascii(get: &dyn Fn(&str) -> Option<String>) -> bool {
    if let Some(v) = get("XAGIT_ASCII") {
        return !matches!(v.as_str(), "0" | "no" | "false");
    }
    if get("TERM").as_deref() == Some("linux") {
        return true;
    }
    if cfg!(target_os = "macos") {
        // Terminal.app and the others are UTF-8 even when LANG is unset
        return false;
    }
    let locale = get("LC_ALL").or_else(|| get("LC_CTYPE")).or_else(|| get("LANG"));
    match locale {
        Some(l) => {
            let l = l.to_ascii_lowercase();
            !(l.contains("utf-8") || l.contains("utf8"))
        }
        None => false,
    }
}

/// macOS' system appearance (`defaults read -g AppleInterfaceStyle` is
/// "Dark" in dark mode and fails in light mode).
fn macos_dark() -> Option<bool> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let out = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().eq_ignore_ascii_case("dark"))
}

impl Theme {
    /// The theme for the terminal xagit runs in.
    pub fn detect() -> Theme {
        let get = |n: &str| env(n);
        let depth = detect_depth(&get);
        let light = depth != Depth::Mono && detect_light(&get, &macos_dark);
        let glyphs = if detect_ascii(&get) { ASCII } else { UNICODE };
        Theme { depth, light, glyphs }
    }

    fn palette(&self) -> &'static Palette {
        if self.light {
            &LIGHT
        } else {
            &DARK
        }
    }

    /// The style of FACE.
    pub fn style(&self, face: Face) -> Style {
        if self.depth == Depth::Mono {
            return mono(face);
        }
        let p = self.palette();
        let basic = self.depth == Depth::Basic;
        // background tints that a 16-color terminal can't show subtly
        let tint = |c: Rgb| if basic { None } else { Some(c) };
        match face {
            Face::Default | Face::Context | Face::PopupDesc => Style::default(),
            Face::Dim | Face::Count | Face::Margin | Face::Fringe | Face::HintText | Face::Completion => Style::fg(p.dim),
            Face::SectionHeading => Style::fg(p.heading).bold(),
            Face::Label => Style::fg(p.dim),
            Face::Hash => Style::fg(p.hash),
            Face::BranchLocal => Style::fg(p.local),
            Face::BranchCurrent => Style::fg(p.local).bold(),
            Face::BranchRemote => Style::fg(p.remote),
            Face::Tag => Style::fg(p.tag),
            Face::Head => Style::fg(p.head).bold(),
            Face::Bisect => Style::fg(p.head),
            Face::Stash => Style::fg(p.stash),
            Face::Refs => Style::fg(p.local),
            Face::Ahead => Style::fg(p.accent),
            Face::Graph => Style::fg(p.dim),
            Face::Keyword => Style::fg(p.keyword),
            // magit-sequence-*: pick inherits default, stop is green, part
            // goldenrod, head sky blue, drop indian red, done/onto magit-hash
            Face::SequencePick => Style::default(),
            Face::SequenceStop => Style::fg(p.local),
            Face::SequencePart => Style::fg(p.tag),
            Face::SequenceHead => Style::fg(p.remote),
            Face::SequenceDrop => Style::fg(p.removed),
            Face::SequenceDone | Face::SequenceOnto => Style::fg(p.hash),
            Face::FileHeading => Style { bold: true, ..Style::default() },
            Face::FileModified => Style::fg(p.modified),
            Face::FileNew => Style::fg(p.added),
            Face::FileDeleted => Style::fg(p.removed),
            Face::FileRenamed => Style::fg(p.remote),
            Face::FileUnmerged => Style::fg(p.conflict).bold(),
            Face::HunkHeading => Style { fg: Some(p.hunk), bg: tint(p.hunk_bg), ..Style::default() },
            Face::HunkRange => Style { fg: Some(p.keyword), bg: tint(p.hunk_bg), bold: true, ..Style::default() },
            Face::Added => Style { fg: Some(p.added), bg: tint(p.added_bg), ..Style::default() },
            Face::Removed => Style { fg: Some(p.removed), bg: tint(p.removed_bg), ..Style::default() },
            Face::Conflict => Style::fg(p.conflict).bold(),
            Face::Meta => Style::fg(p.dim).italic(),
            Face::BlameHeading => Style { fg: Some(p.hunk), bg: tint(p.hunk_bg), ..Style::default() },
            Face::HeaderBar | Face::ModeLine => {
                if basic {
                    Style { reverse: true, ..Style::default() }
                } else {
                    Style::fg(p.bar_fg).on(p.bar_bg)
                }
            }
            Face::HeaderTitle => {
                if basic {
                    Style { reverse: true, bold: true, ..Style::default() }
                } else {
                    Style::fg(p.bar_fg).on(p.bar_bg).bold()
                }
            }
            Face::ModeLineDim => {
                if basic {
                    Style { reverse: true, ..Style::default() }
                } else {
                    Style::fg(p.dim).on(p.bar_bg)
                }
            }
            Face::Message => {
                if basic {
                    Style { reverse: true, ..Style::default() }
                } else {
                    Style::fg(p.bar_fg).on(p.bar_bg)
                }
            }
            Face::Error => {
                if basic {
                    Style { reverse: true, bold: true, ..Style::fg(p.error) }
                } else {
                    Style::fg(p.error).on(p.bar_bg).bold()
                }
            }
            Face::Badge => Style::fg(p.accent_fg).on(p.accent).bold(),
            Face::ModeBadge => Style::fg(p.accent_fg).on(p.mode_bg).bold(),
            Face::HintKey => Style::fg(p.accent).bold(),
            Face::PopupRule => Style::fg(p.dim),
            Face::PopupTitle => Style::fg(p.accent).bold(),
            Face::PopupGroup => Style::fg(p.keyword).bold(),
            Face::PopupKey => Style::fg(p.popup_key).bold(),
            Face::PopupValue => Style::fg(p.added),
            Face::PopupInapt => Style::fg(p.dim).italic(),
            Face::Prompt => Style::fg(p.accent).bold(),
            Face::CursorLine => {
                if basic {
                    Style { bold: true, ..Style::default() }
                } else {
                    Style { bg: Some(p.cursor_bg), ..Style::default() }
                }
            }
            Face::SectionHighlight => Style { bg: tint(p.highlight_bg), ..Style::default() },
            Face::Region => {
                if basic {
                    Style { reverse: true, ..Style::default() }
                } else {
                    Style { bg: Some(p.region_bg), ..Style::default() }
                }
            }
            Face::SearchMatch | Face::Point => {
                if basic {
                    Style { reverse: true, bold: true, ..Style::default() }
                } else {
                    Style::fg(p.accent_fg).on(p.accent).bold()
                }
            }
            Face::Gutter => Style::fg(p.accent).bold(),
        }
    }

    /// The style of the Nth column of a log graph.
    pub fn graph(&self, n: usize) -> Style {
        if self.depth == Depth::Mono {
            return Style::default();
        }
        let g = &self.palette().graph;
        Style::fg(g[n % g.len()])
    }

    /// COLOR as the terminal shows it.
    pub fn color(&self, (r, g, b): Rgb) -> Color {
        match self.depth {
            Depth::True => Color::Rgb { r, g, b },
            Depth::Ansi256 => Color::AnsiValue(ansi256(r, g, b)),
            Depth::Basic | Depth::Mono => basic16(r, g, b),
        }
    }
}

/// Faces without colors: only attributes distinguish them.
fn mono(face: Face) -> Style {
    let bold = Style { bold: true, ..Style::default() };
    let reverse = Style { reverse: true, ..Style::default() };
    match face {
        Face::SectionHeading | Face::FileHeading | Face::HeaderTitle | Face::Badge | Face::ModeBadge => {
            Style { reverse: matches!(face, Face::HeaderTitle | Face::Badge | Face::ModeBadge), ..bold }
        }
        Face::HeaderBar | Face::ModeLine | Face::ModeLineDim | Face::Message | Face::Region => reverse,
        Face::Error => Style { bold: true, ..reverse },
        Face::BranchCurrent | Face::Head | Face::HintKey | Face::PopupKey | Face::PopupTitle | Face::Prompt => bold,
        Face::PopupGroup | Face::HunkRange | Face::Conflict | Face::FileUnmerged | Face::CursorLine | Face::Gutter => bold,
        Face::SearchMatch | Face::Point => reverse,
        Face::PopupValue => Style { underline: true, ..Style::default() },
        Face::Meta | Face::PopupInapt => Style { italic: true, ..Style::default() },
        // keep diff additions / removals distinguishable on a monochrome
        // terminal (the colored styles use green / red backgrounds instead)
        Face::Added => Style { underline: true, ..bold },
        Face::SequenceHead | Face::SequenceStop => bold,
        Face::SequencePart => Style { underline: true, ..Style::default() },
        Face::SequenceDrop => Style { italic: true, ..Style::default() },
        Face::Removed => Style { italic: true, ..Style::default() },
        // Exhaustive on purpose: adding a Face must force a decision about
        // how it looks on a mono terminal, instead of silently becoming
        // plain text.
        Face::Default
        | Face::Dim
        | Face::Count
        | Face::Label
        | Face::Hash
        | Face::BranchLocal
        | Face::BranchRemote
        | Face::Tag
        | Face::Bisect
        | Face::Stash
        | Face::Refs
        | Face::Ahead
        | Face::Graph
        | Face::Keyword
        | Face::SequencePick
        | Face::SequenceDone
        | Face::SequenceOnto
        | Face::FileModified
        | Face::FileNew
        | Face::FileDeleted
        | Face::FileRenamed
        | Face::HunkHeading
        | Face::Context
        | Face::BlameHeading
        | Face::Margin
        | Face::HintText
        | Face::PopupRule
        | Face::PopupDesc
        | Face::Completion
        | Face::SectionHighlight
        | Face::Fringe => Style::default(),
    }
}

/// The xterm-256 index closest to R G B (the 6x6x6 cube or the grays).
pub fn ansi256(r: u8, g: u8, b: u8) -> u8 {
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let near = |v: u8| -> usize {
        STEPS.iter().enumerate().min_by_key(|(_, s)| (i32::from(**s) - i32::from(v)).abs()).map_or(0, |(i, _)| i)
    };
    let (ri, gi, bi) = (near(r), near(g), near(b));
    let cube = (STEPS[ri], STEPS[gi], STEPS[bi]);
    let avg = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    let gray_i = if avg < 8 { 0 } else { ((avg - 8) / 10).min(23) as u8 };
    let gray_v = 8 + 10 * gray_i;
    let dist = |(a, b2, c): (u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a, r) + d(b2, g) + d(c, b)
    };
    if dist((gray_v, gray_v, gray_v)) < dist(cube) {
        232 + gray_i
    } else {
        16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8
    }
}

/// The closest of the 16 ANSI colors (by hue, as their actual values
/// differ between terminals).
pub fn basic16(r: u8, g: u8, b: u8) -> Color {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let bright = max > 0xB0;
    if max - min < 0x28 {
        return match max {
            0..=0x3F => Color::Black,
            0x40..=0x9F => Color::DarkGrey,
            0xA0..=0xD7 => Color::Grey,
            _ => Color::White,
        };
    }
    let has = |c: i32| c - min >= (max - min) / 2;
    match (has(r), has(g), has(b)) {
        (true, false, false) => if bright { Color::Red } else { Color::DarkRed },
        (false, true, false) => if bright { Color::Green } else { Color::DarkGreen },
        (false, false, true) => if bright { Color::Blue } else { Color::DarkBlue },
        (true, true, false) => if bright { Color::Yellow } else { Color::DarkYellow },
        (false, true, true) => if bright { Color::Cyan } else { Color::DarkCyan },
        (true, false, true) => if bright { Color::Magenta } else { Color::DarkMagenta },
        _ => Color::Grey,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn envs(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| m.get(k).cloned()
    }

    #[test]
    fn depth_detection() {
        assert_eq!(detect_depth(&envs(&[("TERM", "xterm-256color"), ("COLORTERM", "truecolor")])), Depth::True);
        assert_eq!(detect_depth(&envs(&[("TERM", "xterm-256color"), ("TERM_PROGRAM", "Apple_Terminal")])), Depth::Ansi256);
        assert_eq!(detect_depth(&envs(&[("TERM", "xterm-256color"), ("TERM_PROGRAM", "iTerm.app")])), Depth::True);
        assert_eq!(detect_depth(&envs(&[("TERM", "xterm-kitty")])), Depth::True);
        assert_eq!(detect_depth(&envs(&[("TERM", "xterm-256color"), ("VTE_VERSION", "7200")])), Depth::True);
        assert_eq!(detect_depth(&envs(&[("TERM", "linux")])), Depth::Basic);
        assert_eq!(detect_depth(&envs(&[("TERM", "screen")])), Depth::Ansi256);
        assert_eq!(detect_depth(&envs(&[("TERM", "dumb")])), Depth::Mono);
        assert_eq!(detect_depth(&envs(&[("TERM", "xterm-256color"), ("NO_COLOR", "1")])), Depth::Mono);
        assert_eq!(detect_depth(&envs(&[("NO_COLOR", "1"), ("XAGIT_COLOR", "256")])), Depth::Ansi256);
        assert_eq!(detect_depth(&envs(&[("TERM", "xterm-256color"), ("XAGIT_COLOR", "never")])), Depth::Mono);
    }

    #[test]
    fn background_detection() {
        let none = || None;
        assert!(!detect_light(&envs(&[]), &none));
        assert!(detect_light(&envs(&[("COLORFGBG", "0;15")]), &none));
        assert!(!detect_light(&envs(&[("COLORFGBG", "15;default;0")]), &none));
        assert!(detect_light(&envs(&[("XAGIT_THEME", "light"), ("COLORFGBG", "15;0")]), &none));
        let light_mode = || Some(false);
        assert!(detect_light(&envs(&[("TERM_PROGRAM", "Apple_Terminal")]), &light_mode));
        assert!(!detect_light(&envs(&[("TERM_PROGRAM", "iTerm.app")]), &light_mode));
    }

    #[test]
    fn ascii_glyphs() {
        assert!(detect_ascii(&envs(&[("TERM", "linux")])));
        assert!(detect_ascii(&envs(&[("XAGIT_ASCII", "1")])));
        assert!(!detect_ascii(&envs(&[("TERM", "xterm-256color"), ("LANG", "en_US.UTF-8")])));
        if !cfg!(target_os = "macos") {
            assert!(detect_ascii(&envs(&[("TERM", "xterm"), ("LC_ALL", "C")])));
        }
    }

    #[test]
    fn color_reduction() {
        assert_eq!(ansi256(0, 0, 0), 16);
        assert_eq!(ansi256(255, 255, 255), 231);
        assert_eq!(ansi256(0x80, 0x80, 0x80), 244);
        assert_eq!(ansi256(0xFF, 0x00, 0x00), 196);
        assert_eq!(basic16(0xE7, 0x82, 0x84), Color::Red);
        assert_eq!(basic16(0xF2, 0xA6, 0x5A), Color::Yellow);
        assert_eq!(basic16(0xB4, 0xBE, 0xFE), Color::Blue);
        assert_eq!(basic16(0x2E, 0x7D, 0x32), Color::DarkGreen);
        assert_eq!(basic16(0x7C, 0x80, 0x8C), Color::DarkGrey);
    }

    #[test]
    fn basic_terminals_get_no_background_tints() {
        let t = Theme { depth: Depth::Basic, light: false, glyphs: ASCII };
        assert_eq!(t.style(Face::Added).bg, None);
        assert_eq!(t.style(Face::SectionHighlight).bg, None);
        let t = Theme { depth: Depth::True, light: false, glyphs: UNICODE };
        assert!(t.style(Face::Added).bg.is_some());
        assert_eq!(t.style(Face::Default), Style::default());
    }
}

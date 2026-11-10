//! ui/width.rs — the number of terminal columns text takes.
//!
//! Terminals on Linux and macOS draw East Asian wide characters (CJK,
//! Hangul, fullwidth forms) and most emoji in two columns, and combining
//! marks in none. Counting `char`s instead overflows rows that contain
//! Chinese commit messages or author names, and misaligns the margins.

/// Columns between tab stops (the terminal default on both platforms).
pub const TAB_WIDTH: usize = 8;

const WIDE: &[(u32, u32)] = &[
    (0x1100, 0x115F),
    (0x231A, 0x231B),
    (0x2329, 0x232A),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x2E80, 0x303E),
    (0x3041, 0x33FF),
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xA000, 0xA4CF),
    (0xA960, 0xA97F),
    (0xAC00, 0xD7A3),
    (0xF900, 0xFAFF),
    (0xFE10, 0xFE19),
    (0xFE30, 0xFE6F),
    (0xFF00, 0xFF60),
    (0xFFE0, 0xFFE6),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F200, 0x1F202),
    (0x1F210, 0x1F23B),
    (0x1F240, 0x1F248),
    (0x1F250, 0x1F251),
    (0x1F260, 0x1F265),
    (0x1F300, 0x1F64F),
    (0x1F680, 0x1F6FF),
    (0x1F7E0, 0x1F7EB),
    (0x1F90C, 0x1F9FF),
    (0x1FA70, 0x1FAFF),
    (0x20000, 0x2FFFD),
    (0x30000, 0x3FFFD),
];

const ZERO: &[(u32, u32)] = &[
    (0x0300, 0x036F),
    (0x0483, 0x0489),
    (0x0591, 0x05BD),
    (0x0610, 0x061A),
    (0x064B, 0x065F),
    (0x0670, 0x0670),
    (0x06D6, 0x06DC),
    (0x06DF, 0x06E4),
    (0x06E7, 0x06E8),
    (0x06EA, 0x06ED),
    (0x0E31, 0x0E31),
    (0x0E34, 0x0E3A),
    (0x0E47, 0x0E4E),
    (0x1AB0, 0x1AFF),
    (0x1DC0, 0x1DFF),
    (0x200B, 0x200F),
    (0x2028, 0x202E),
    (0x2060, 0x2064),
    (0x20D0, 0x20FF),
    (0xFE00, 0xFE0F),
    (0xFE20, 0xFE2F),
    (0xFEFF, 0xFEFF),
    (0x1F3FB, 0x1F3FF),
    (0xE0100, 0xE01EF),
];

fn in_table(table: &[(u32, u32)], c: u32) -> bool {
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < c {
                std::cmp::Ordering::Less
            } else if lo > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Columns of C (control characters other than tab count as none; the
/// painter shows them in caret notation).
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    if u < 0x20 || (0x7F..0xA0).contains(&u) {
        0
    } else if u < 0x300 {
        1
    } else if in_table(ZERO, u) {
        0
    } else if in_table(WIDE, u) {
        2
    } else {
        1
    }
}

/// Columns of S, without tab expansion.
pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// The longest prefix of S that fits in WIDTH columns. A wide character
/// that would straddle the edge is left out.
pub fn truncate(s: &str, width: usize) -> &str {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        let cw = char_width(c);
        if used + cw > width {
            return &s[..i];
        }
        used += cw;
    }
    s
}

/// S cut to WIDTH columns, then padded with spaces to exactly WIDTH.
pub fn fit(s: &str, width: usize) -> String {
    let t = truncate(s, width);
    let pad = width - str_width(t);
    format!("{t}{}", " ".repeat(pad))
}

/// S cut to WIDTH columns, its last column an ellipsis when cut.
pub fn ellipsize(s: &str, width: usize) -> String {
    if str_width(s) <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    format!("{}…", truncate(s, width - 1))
}

/// S padded with spaces on the right to WIDTH columns (never cut).
pub fn pad_right(s: &str, width: usize) -> String {
    let w = str_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

/// Make S safe and measurable for a terminal starting at column COL:
/// tabs become spaces to the next tab stop, other control characters
/// are shown in caret notation (^M, ^[) as Emacs does, so that git
/// output can neither move the cursor nor inject escape sequences.
/// Returns the text and the column after it.
pub fn sanitize_for_terminal(s: &str, col: usize) -> (String, usize) {
    if !s.chars().any(|c| c == '\t' || char_width(c) == 0 && (c as u32) < 0xA0) {
        return (s.to_string(), col + str_width(s));
    }
    let mut out = String::with_capacity(s.len());
    let mut col = col;
    for c in s.chars() {
        let u = c as u32;
        if c == '\t' {
            let n = TAB_WIDTH - col % TAB_WIDTH;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else if u < 0x20 || u == 0x7F {
            out.push('^');
            out.push(char::from_u32(u ^ 0x40).unwrap_or('?'));
            col += 2;
        } else if (0x80..0xA0).contains(&u) {
            out.push_str(&format!("\\{u:o}"));
            col += 4;
        } else {
            out.push(c);
            col += char_width(c);
        }
    }
    (out, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(str_width("main"), 4);
        assert_eq!(str_width("修复登录"), 8);
        assert_eq!(str_width("ｆｕｌｌ"), 8);
        assert_eq!(str_width("e\u{301}"), 1);
        assert_eq!(str_width("🚀"), 2);
        assert_eq!(str_width("한국"), 4);
        assert_eq!(str_width("▸▾▌│"), 4);
    }

    #[test]
    fn truncation_never_splits_a_wide_char() {
        assert_eq!(truncate("修复登录", 5), "修复");
        assert_eq!(truncate("ab修复", 3), "ab");
        assert_eq!(fit("修复", 5), "修复 ");
        assert_eq!(fit("abcdef", 3), "abc");
        assert_eq!(ellipsize("张三丰大侠", 5), "张三…");
        assert_eq!(ellipsize("abc", 5), "abc");
        assert_eq!(pad_right("张三", 6), "张三  ");
    }

    #[test]
    fn tabs_and_control_characters() {
        assert_eq!(sanitize_for_terminal("a\tb", 0), ("a       b".to_string(), 9));
        assert_eq!(sanitize_for_terminal("\tx", 3), ("     x".to_string(), 9));
        assert_eq!(sanitize_for_terminal("ok\r", 0), ("ok^M".to_string(), 4));
        assert_eq!(sanitize_for_terminal("\u{1b}[31m", 0).0, "^[[31m");
        assert_eq!(sanitize_for_terminal("plain", 2), ("plain".to_string(), 7));
    }
}

//! Build an xkb keymap on the fly so a virtual keyboard can type arbitrary
//! Unicode text: each distinct character gets its own keycode (the wtype
//! trick). A fixed set of named keys (Return, Escape, modifiers, arrows…)
//! is always included so key combos work with the same keymap.

use std::collections::BTreeMap;

/// Modifier bit for the `modifiers` request, by named key.
pub fn modifier_mask(name: &str) -> Option<u32> {
    match name {
        "shift" => Some(1),
        "ctrl" | "control" => Some(4),
        "alt" => Some(8),
        "super" | "meta" => Some(64),
        _ => None,
    }
}

/// evdev code of the no-op key present in every generated keymap.
pub const VOID_CODE: u32 = 1;

/// A fully expanded standard US keymap (`xkbcli compile-keymap --layout us`).
/// Used for ASCII text and key combos so apps see the keycodes a real keyboard
/// would send; some (Firefox) ignore text from keycodes they do not recognise.
pub const US_KEYMAP: &str = include_str!("us.xkb");

/// evdev keycode and whether Shift is needed, for a character on the US layout.
pub fn us_key(c: char) -> Option<(u32, bool)> {
    let lower = |code| Some((code, false));
    let upper = |code| Some((code, true));
    match c {
        'a'..='z' | 'A'..='Z' => {
            let code = match c.to_ascii_lowercase() {
                'q' => 16,
                'w' => 17,
                'e' => 18,
                'r' => 19,
                't' => 20,
                'y' => 21,
                'u' => 22,
                'i' => 23,
                'o' => 24,
                'p' => 25,
                'a' => 30,
                's' => 31,
                'd' => 32,
                'f' => 33,
                'g' => 34,
                'h' => 35,
                'j' => 36,
                'k' => 37,
                'l' => 38,
                'z' => 44,
                'x' => 45,
                'c' => 46,
                'v' => 47,
                'b' => 48,
                'n' => 49,
                'm' => 50,
                _ => return None,
            };
            Some((code, c.is_ascii_uppercase()))
        }
        '1' => lower(2),
        '2' => lower(3),
        '3' => lower(4),
        '4' => lower(5),
        '5' => lower(6),
        '6' => lower(7),
        '7' => lower(8),
        '8' => lower(9),
        '9' => lower(10),
        '0' => lower(11),
        '!' => upper(2),
        '@' => upper(3),
        '#' => upper(4),
        '$' => upper(5),
        '%' => upper(6),
        '^' => upper(7),
        '&' => upper(8),
        '*' => upper(9),
        '(' => upper(10),
        ')' => upper(11),
        '-' => lower(12),
        '_' => upper(12),
        '=' => lower(13),
        '+' => upper(13),
        '[' => lower(26),
        '{' => upper(26),
        ']' => lower(27),
        '}' => upper(27),
        '\\' => lower(43),
        '|' => upper(43),
        ';' => lower(39),
        ':' => upper(39),
        '\'' => lower(40),
        '"' => upper(40),
        '`' => lower(41),
        '~' => upper(41),
        ',' => lower(51),
        '<' => upper(51),
        '.' => lower(52),
        '>' => upper(52),
        '/' => lower(53),
        '?' => upper(53),
        ' ' => lower(57),
        '\n' => lower(28),
        '\t' => lower(15),
        _ => None,
    }
}

/// evdev keycode of a named key on the US layout.
pub fn us_named(name: &str) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "return" | "enter" => 28,
        "tab" => 15,
        "escape" | "esc" => 1,
        "backspace" => 14,
        "delete" => 111,
        "space" => 57,
        "up" => 103,
        "down" => 108,
        "left" => 105,
        "right" => 106,
        "home" => 102,
        "end" => 107,
        "pageup" => 104,
        "pagedown" => 109,
        "insert" => 110,
        "ctrl" | "control" => 29,
        "shift" => 42,
        "alt" => 56,
        "super" | "meta" => 125,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "f11" => 87,
        "f12" => 88,
        _ => return None,
    })
}

/// A harmless key on the US layout to absorb the post-keymap-change drop (Left Shift).
pub const US_ABSORB_CODE: u32 = 42;

/// The xkb keysym name for a character. ASCII gets the standard Latin-1 keysyms
/// (what a real keyboard produces), everything else the Unicode form `UXXXX`.
/// Some toolkits treat Unicode keysyms differently from Latin-1 ones for text input.
pub fn keysym_name(c: char) -> String {
    if c.is_ascii_alphanumeric() {
        return c.to_string();
    }
    let named = match c {
        ' ' => "space",
        '!' => "exclam",
        '"' => "quotedbl",
        '#' => "numbersign",
        '$' => "dollar",
        '%' => "percent",
        '&' => "ampersand",
        '\'' => "apostrophe",
        '(' => "parenleft",
        ')' => "parenright",
        '*' => "asterisk",
        '+' => "plus",
        ',' => "comma",
        '-' => "minus",
        '.' => "period",
        '/' => "slash",
        ':' => "colon",
        ';' => "semicolon",
        '<' => "less",
        '=' => "equal",
        '>' => "greater",
        '?' => "question",
        '@' => "at",
        '[' => "bracketleft",
        '\\' => "backslash",
        ']' => "bracketright",
        '^' => "asciicircum",
        '_' => "underscore",
        '`' => "grave",
        '{' => "braceleft",
        '|' => "bar",
        '}' => "braceright",
        '~' => "asciitilde",
        _ => return format!("U{:04X}", c as u32),
    };
    named.to_string()
}

pub struct Keymap {
    pub text: String,
    /// evdev key code for each character.
    chars: BTreeMap<char, u32>,
}

impl Keymap {
    /// Build a keymap covering every character in `text` plus the named keys.
    pub fn for_text(text: &str) -> Self {
        let mut chars = BTreeMap::new();
        let mut symbols = String::new();
        let mut codes = String::new();
        // Code 1 is a deliberate no-op key: pressed once after every keymap upload,
        // because the first key event after a keymap change gets dropped on the way
        // to the client (observed on sway 1.12).
        codes.push_str(&format!("        <K{VOID_CODE}> = {};\n", VOID_CODE + 8));
        symbols.push_str(&format!(
            "        key <K{VOID_CODE}> {{ [ VoidSymbol ] }};\n"
        ));
        let mut next: u32 = VOID_CODE + 1; // evdev code; xkb keycode = code + 8

        let mut unique: Vec<char> = text
            .chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .collect();
        unique.sort_unstable();
        unique.dedup();
        for c in unique {
            let sym = match c {
                '\n' => "Return".to_string(),
                '\t' => "Tab".to_string(),
                _ => keysym_name(c),
            };
            let code = next;
            next += 1;
            codes.push_str(&format!("        <K{code}> = {};\n", code + 8));
            symbols.push_str(&format!("        key <K{code}> {{ [ {sym} ] }};\n"));
            chars.insert(c, code);
        }
        let max = next + 8;
        let modmap = String::new();
        let text = format!(
            "xkb_keymap {{\n    xkb_keycodes \"slate\" {{\n        minimum = 8;\n        maximum = {max};\n{codes}    }};\n    xkb_types \"slate\" {{ include \"complete\" }};\n    xkb_compatibility \"slate\" {{ include \"complete\" }};\n    xkb_symbols \"slate\" {{\n{symbols}{modmap}    }};\n}};\n"
        );
        Self { text, chars }
    }

    pub fn chars_iter(&self) -> impl Iterator<Item = char> + '_ {
        self.chars.keys().copied()
    }

    pub fn code_for_char(&self, c: char) -> Option<u32> {
        self.chars.get(&c).copied()
    }
}

/// Parse "ctrl+shift+t" into (modifier names, key name). A single character
/// key like "t" is returned as-is and typed through the char table.
pub fn parse_combo(combo: &str) -> (Vec<String>, String) {
    let parts: Vec<&str> = combo
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return (vec![], String::new());
    }
    let key = parts[parts.len() - 1].to_string();
    let mods = parts[..parts.len() - 1]
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    (mods, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_keymap_with_unicode() {
        let k = Keymap::for_text("hi 你");
        assert!(k.text.contains("U4F60"));
        assert!(k.text.contains("[ space ]"));
        assert!(k.text.contains("[ h ]"));
        assert_eq!(keysym_name('/'), "slash");
        assert_eq!(keysym_name('é'), "U00E9");
        assert!(k.code_for_char('h').is_some());
    }

    #[test]
    fn us_table() {
        assert_eq!(us_key('a'), Some((30, false)));
        assert_eq!(us_key('A'), Some((30, true)));
        assert_eq!(us_key('/'), Some((53, false)));
        assert_eq!(us_key('?'), Some((53, true)));
        assert_eq!(us_key('你'), None);
        assert_eq!(us_named("Return"), Some(28));
        assert!(US_KEYMAP.contains("xkb_keymap"));
    }

    #[test]
    fn parses_combos() {
        assert_eq!(
            parse_combo("ctrl+shift+t"),
            (vec!["ctrl".into(), "shift".into()], "t".into())
        );
        assert_eq!(parse_combo("Return"), (vec![], "Return".into()));
    }
}

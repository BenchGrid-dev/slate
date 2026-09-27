//! Build an xkb keymap on the fly so a virtual keyboard can type arbitrary
//! Unicode text: each distinct character gets its own keycode (the wtype
//! trick). A fixed set of named keys (Return, Escape, modifiers, arrows…)
//! is always included so key combos work with the same keymap.

use std::collections::BTreeMap;

/// Named keys always present, in keycode order after the text characters.
pub const NAMED_KEYS: &[(&str, &str)] = &[
    ("return", "Return"),
    ("enter", "Return"),
    ("tab", "Tab"),
    ("escape", "Escape"),
    ("esc", "Escape"),
    ("backspace", "BackSpace"),
    ("delete", "Delete"),
    ("space", "space"),
    ("up", "Up"),
    ("down", "Down"),
    ("left", "Left"),
    ("right", "Right"),
    ("home", "Home"),
    ("end", "End"),
    ("pageup", "Prior"),
    ("pagedown", "Next"),
    ("insert", "Insert"),
    ("ctrl", "Control_L"),
    ("control", "Control_L"),
    ("shift", "Shift_L"),
    ("alt", "Alt_L"),
    ("super", "Super_L"),
    ("meta", "Super_L"),
    ("f1", "F1"),
    ("f2", "F2"),
    ("f3", "F3"),
    ("f4", "F4"),
    ("f5", "F5"),
    ("f6", "F6"),
    ("f7", "F7"),
    ("f8", "F8"),
    ("f9", "F9"),
    ("f10", "F10"),
    ("f11", "F11"),
    ("f12", "F12"),
];

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

/// evdev code of the no-op key present in every keymap.
pub const VOID_CODE: u32 = 1;

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
    /// evdev key code for each named key (lowercase name).
    named: BTreeMap<String, u32>,
}

impl Keymap {
    /// Build a keymap covering every character in `text` plus the named keys.
    pub fn for_text(text: &str) -> Self {
        let mut chars = BTreeMap::new();
        let mut named = BTreeMap::new();
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
        // Named keys, deduplicated by keysym so aliases share a code.
        let mut by_sym: BTreeMap<&str, u32> = BTreeMap::new();
        for (name, sym) in NAMED_KEYS {
            let code = match by_sym.get(sym) {
                Some(c) => *c,
                None => {
                    let code = next;
                    next += 1;
                    codes.push_str(&format!("        <K{code}> = {};\n", code + 8));
                    symbols.push_str(&format!("        key <K{code}> {{ [ {sym} ] }};\n"));
                    by_sym.insert(sym, code);
                    code
                }
            };
            named.insert((*name).to_string(), code);
        }
        let max = next + 8;
        // Bind the modifier keysyms to real modifiers, or clients computing state from key
        // events (rather than our modifiers request) would never see Control/Shift held.
        let mut modmap = String::new();
        for (sym, modname) in [
            ("Control_L", "Control"),
            ("Shift_L", "Shift"),
            ("Alt_L", "Mod1"),
            ("Super_L", "Mod4"),
        ] {
            if let Some(code) = by_sym.get(sym) {
                modmap.push_str(&format!(
                    "        modifier_map {modname} {{ <K{code}> }};\n"
                ));
            }
        }
        let text = format!(
            "xkb_keymap {{\n    xkb_keycodes \"slate\" {{\n        minimum = 8;\n        maximum = {max};\n{codes}    }};\n    xkb_types \"slate\" {{ include \"complete\" }};\n    xkb_compatibility \"slate\" {{ include \"complete\" }};\n    xkb_symbols \"slate\" {{\n{symbols}{modmap}    }};\n}};\n"
        );
        Self { text, chars, named }
    }

    pub fn chars_iter(&self) -> impl Iterator<Item = char> + '_ {
        self.chars.keys().copied()
    }

    pub fn code_for_char(&self, c: char) -> Option<u32> {
        self.chars.get(&c).copied()
    }

    pub fn code_for_named(&self, name: &str) -> Option<u32> {
        self.named.get(&name.to_ascii_lowercase()).copied()
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
        assert!(k.text.contains("modifier_map Control"));
        assert!(k.code_for_char('h').is_some());
        assert!(k.code_for_named("Return").is_some());
        assert_eq!(k.code_for_named("enter"), k.code_for_named("return"));
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

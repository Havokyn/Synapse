use synapse_core::{Key, KeyCode};

use crate::ActionError;

pub(super) fn hid_usage(key: &Key) -> Result<u8, ActionError> {
    match &key.code {
        KeyCode::HidCode { value } => hid_code_usage(*value),
        KeyCode::Symbol { value } => symbol_usage(*value).ok_or_else(|| unsupported_key(key)),
        KeyCode::Named { value } => named_usage(value).ok_or_else(|| unsupported_key(key)),
    }
}

pub(super) fn hid_usage_for_text_char(ch: char) -> Result<u8, ActionError> {
    unmodified_text_usage(ch).ok_or_else(|| ActionError::UnsupportedKey {
        detail: format!("hardware backend cannot type non-US-layout character {ch:?}"),
    })
}

fn hid_code_usage(value: u8) -> Result<u8, ActionError> {
    if is_defined_keyboard_usage(value) {
        Ok(value)
    } else {
        Err(ActionError::UnsupportedKey {
            detail: format!(
                "hardware HID usage code 0x{value:02X} is not a defined keyboard/keypad key"
            ),
        })
    }
}

fn named_usage(value: &str) -> Option<u8> {
    let trimmed = value.trim();
    let mut chars = trimmed.chars();
    if let Some(ch) = chars.next()
        && chars.next().is_none()
    {
        return symbol_usage(ch);
    }

    let normalized = normalize_name(trimmed);
    alias_usage(&normalized)
        .or_else(|| generated_name_usage(&normalized))
        .or_else(|| table_name_usage(&normalized))
}

const fn symbol_usage(value: char) -> Option<u8> {
    match value {
        'a' | 'A' => Some(0x04),
        'b' | 'B' => Some(0x05),
        'c' | 'C' => Some(0x06),
        'd' | 'D' => Some(0x07),
        'e' | 'E' => Some(0x08),
        'f' | 'F' => Some(0x09),
        'g' | 'G' => Some(0x0A),
        'h' | 'H' => Some(0x0B),
        'i' | 'I' => Some(0x0C),
        'j' | 'J' => Some(0x0D),
        'k' | 'K' => Some(0x0E),
        'l' | 'L' => Some(0x0F),
        'm' | 'M' => Some(0x10),
        'n' | 'N' => Some(0x11),
        'o' | 'O' => Some(0x12),
        'p' | 'P' => Some(0x13),
        'q' | 'Q' => Some(0x14),
        'r' | 'R' => Some(0x15),
        's' | 'S' => Some(0x16),
        't' | 'T' => Some(0x17),
        'u' | 'U' => Some(0x18),
        'v' | 'V' => Some(0x19),
        'w' | 'W' => Some(0x1A),
        'x' | 'X' => Some(0x1B),
        'y' | 'Y' => Some(0x1C),
        'z' | 'Z' => Some(0x1D),
        '1' | '!' => Some(0x1E),
        '2' | '@' => Some(0x1F),
        '3' | '#' => Some(0x20),
        '4' | '$' => Some(0x21),
        '5' | '%' => Some(0x22),
        '6' | '^' => Some(0x23),
        '7' | '&' => Some(0x24),
        '8' | '*' => Some(0x25),
        '9' | '(' => Some(0x26),
        '0' | ')' => Some(0x27),
        '\n' | '\r' => Some(0x28),
        '\t' => Some(0x2B),
        ' ' => Some(0x2C),
        '-' | '_' => Some(0x2D),
        '=' | '+' => Some(0x2E),
        '[' | '{' => Some(0x2F),
        ']' | '}' => Some(0x30),
        '\\' | '|' => Some(0x31),
        ';' | ':' => Some(0x33),
        '\'' | '"' => Some(0x34),
        '`' | '~' => Some(0x35),
        ',' | '<' => Some(0x36),
        '.' | '>' => Some(0x37),
        '/' | '?' => Some(0x38),
        _ => None,
    }
}

const fn unmodified_text_usage(value: char) -> Option<u8> {
    match value {
        'A'..='Z'
        | '!'
        | '@'
        | '#'
        | '$'
        | '%'
        | '^'
        | '&'
        | '*'
        | '('
        | ')'
        | '_'
        | '+'
        | '{'
        | '}'
        | '|'
        | ':'
        | '"'
        | '~'
        | '<'
        | '>'
        | '?' => None,
        _ => symbol_usage(value),
    }
}

fn generated_name_usage(name: &str) -> Option<u8> {
    keyboard_letter_name(name)
        .or_else(|| keyboard_digit_name(name))
        .or_else(|| function_key_name(name))
        .or_else(|| international_name(name))
        .or_else(|| lang_name(name))
        .or_else(|| keypad_digit_name(name))
}

fn keyboard_letter_name(name: &str) -> Option<u8> {
    let suffix = name.strip_prefix("keyboard")?;
    single_ascii_letter(suffix).map(|letter| 0x04 + (letter - b'a'))
}

fn keyboard_digit_name(name: &str) -> Option<u8> {
    let suffix = name.strip_prefix("keyboard")?;
    keyboard_digit_usage(suffix)
}

fn function_key_name(name: &str) -> Option<u8> {
    let suffix = name
        .strip_prefix("keyboardf")
        .or_else(|| name.strip_prefix('f'))?;
    let number = suffix.parse::<u8>().ok()?;
    match number {
        1..=12 => Some(0x3A + number - 1),
        13..=24 => Some(0x68 + number - 13),
        _ => None,
    }
}

fn international_name(name: &str) -> Option<u8> {
    let suffix = name.strip_prefix("keyboardinternational")?;
    let number = suffix.parse::<u8>().ok()?;
    (1..=9).contains(&number).then_some(0x87 + number - 1)
}

fn lang_name(name: &str) -> Option<u8> {
    let suffix = name.strip_prefix("keyboardlang")?;
    let number = suffix.parse::<u8>().ok()?;
    (1..=9).contains(&number).then_some(0x90 + number - 1)
}

fn keypad_digit_name(name: &str) -> Option<u8> {
    ["keypad", "numpad", "kp"]
        .into_iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .and_then(keypad_digit_usage)
}

fn keyboard_digit_usage(value: &str) -> Option<u8> {
    match value {
        "1" => Some(0x1E),
        "2" => Some(0x1F),
        "3" => Some(0x20),
        "4" => Some(0x21),
        "5" => Some(0x22),
        "6" => Some(0x23),
        "7" => Some(0x24),
        "8" => Some(0x25),
        "9" => Some(0x26),
        "0" => Some(0x27),
        _ => None,
    }
}

fn keypad_digit_usage(value: &str) -> Option<u8> {
    match value {
        "1" => Some(0x59),
        "2" => Some(0x5A),
        "3" => Some(0x5B),
        "4" => Some(0x5C),
        "5" => Some(0x5D),
        "6" => Some(0x5E),
        "7" => Some(0x5F),
        "8" => Some(0x60),
        "9" => Some(0x61),
        "0" => Some(0x62),
        _ => None,
    }
}

fn single_ascii_letter(value: &str) -> Option<u8> {
    let bytes = value.as_bytes();
    (bytes.len() == 1 && bytes[0].is_ascii_lowercase()).then_some(bytes[0])
}

fn alias_usage(name: &str) -> Option<u8> {
    HID_USAGE_ALIASES
        .iter()
        .find_map(|(alias, usage)| (*alias == name).then_some(*usage))
}

fn table_name_usage(name: &str) -> Option<u8> {
    HID_USAGE_NAMES
        .iter()
        .find_map(|(known, usage)| (*known == name).then_some(*usage))
}

const fn is_defined_keyboard_usage(value: u8) -> bool {
    matches!(value, 0x04..=0xA4 | 0xB0..=0xDD | 0xE0..=0xE7)
}

fn normalize_name(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn unsupported_key(key: &Key) -> ActionError {
    ActionError::UnsupportedKey {
        detail: format!(
            "hardware backend does not support key code {:?} as a USB HID keyboard/keypad usage",
            key.code
        ),
    }
}

const HID_USAGE_ALIASES: &[(&str, u8)] = &[
    ("enter", 0x28),
    ("return", 0x28),
    ("esc", 0x29),
    ("escape", 0x29),
    ("backspace", 0x2A),
    ("deletebackward", 0x2A),
    ("tab", 0x2B),
    ("space", 0x2C),
    ("spacebar", 0x2C),
    ("capslock", 0x39),
    ("caps", 0x39),
    ("printscreen", 0x46),
    ("printscr", 0x46),
    ("prtsc", 0x46),
    ("scrolllock", 0x47),
    ("pause", 0x48),
    ("break", 0x48),
    ("insert", 0x49),
    ("ins", 0x49),
    ("home", 0x4A),
    ("pageup", 0x4B),
    ("pgup", 0x4B),
    ("delete", 0x4C),
    ("del", 0x4C),
    ("end", 0x4D),
    ("pagedown", 0x4E),
    ("pgdn", 0x4E),
    ("right", 0x4F),
    ("arrowright", 0x4F),
    ("left", 0x50),
    ("arrowleft", 0x50),
    ("down", 0x51),
    ("arrowdown", 0x51),
    ("up", 0x52),
    ("arrowup", 0x52),
    ("numlock", 0x53),
    ("application", 0x65),
    ("apps", 0x65),
    ("contextmenu", 0x65),
    ("power", 0x66),
    ("execute", 0x74),
    ("help", 0x75),
    ("menu", 0x76),
    ("select", 0x77),
    ("stop", 0x78),
    ("again", 0x79),
    ("undo", 0x7A),
    ("cut", 0x7B),
    ("copy", 0x7C),
    ("paste", 0x7D),
    ("find", 0x7E),
    ("mute", 0x7F),
    ("volumeup", 0x80),
    ("volup", 0x80),
    ("volumedown", 0x81),
    ("voldown", 0x81),
    ("ctrl", 0xE0),
    ("control", 0xE0),
    ("leftctrl", 0xE0),
    ("leftcontrol", 0xE0),
    ("lctrl", 0xE0),
    ("lcontrol", 0xE0),
    ("shift", 0xE1),
    ("leftshift", 0xE1),
    ("lshift", 0xE1),
    ("alt", 0xE2),
    ("option", 0xE2),
    ("leftalt", 0xE2),
    ("leftoption", 0xE2),
    ("lalt", 0xE2),
    ("loption", 0xE2),
    ("meta", 0xE3),
    ("win", 0xE3),
    ("windows", 0xE3),
    ("super", 0xE3),
    ("command", 0xE3),
    ("cmd", 0xE3),
    ("leftmeta", 0xE3),
    ("leftwin", 0xE3),
    ("leftgui", 0xE3),
    ("rightctrl", 0xE4),
    ("rightcontrol", 0xE4),
    ("rctrl", 0xE4),
    ("rcontrol", 0xE4),
    ("rightshift", 0xE5),
    ("rshift", 0xE5),
    ("rightalt", 0xE6),
    ("rightoption", 0xE6),
    ("ralt", 0xE6),
    ("roption", 0xE6),
    ("rightmeta", 0xE7),
    ("rightwin", 0xE7),
    ("rightgui", 0xE7),
    ("rwin", 0xE7),
    ("rgui", 0xE7),
];

const HID_USAGE_NAMES: &[(&str, u8)] = &[
    ("keyboardenter", 0x28),
    ("keyboardescape", 0x29),
    ("keyboarddeletebackspace", 0x2A),
    ("keyboardtab", 0x2B),
    ("keyboardspacebar", 0x2C),
    ("keyboardminus", 0x2D),
    ("keyboardequal", 0x2E),
    ("keyboardleftbracket", 0x2F),
    ("keyboardrightbracket", 0x30),
    ("keyboardbackslash", 0x31),
    ("keyboardnonushash", 0x32),
    ("keyboardsemicolon", 0x33),
    ("keyboardquote", 0x34),
    ("keyboardgraveaccent", 0x35),
    ("keyboardcomma", 0x36),
    ("keyboardperiod", 0x37),
    ("keyboardslash", 0x38),
    ("keyboardcapslock", 0x39),
    ("keyboardprintscreen", 0x46),
    ("keyboardscrolllock", 0x47),
    ("keyboardpause", 0x48),
    ("keyboardinsert", 0x49),
    ("keyboardhome", 0x4A),
    ("keyboardpageup", 0x4B),
    ("keyboarddeleteforward", 0x4C),
    ("keyboardend", 0x4D),
    ("keyboardpagedown", 0x4E),
    ("keyboardrightarrow", 0x4F),
    ("keyboardleftarrow", 0x50),
    ("keyboarddownarrow", 0x51),
    ("keyboarduparrow", 0x52),
    ("keypadnumlockandclear", 0x53),
    ("keypadslash", 0x54),
    ("keypadasterisk", 0x55),
    ("keypadminus", 0x56),
    ("keypadplus", 0x57),
    ("keypadenter", 0x58),
    ("keypadperiod", 0x63),
    ("keyboardnonusbackslash", 0x64),
    ("keyboardapplication", 0x65),
    ("keyboardpower", 0x66),
    ("keypadequal", 0x67),
    ("keyboardexecute", 0x74),
    ("keyboardhelp", 0x75),
    ("keyboardmenu", 0x76),
    ("keyboardselect", 0x77),
    ("keyboardstop", 0x78),
    ("keyboardagain", 0x79),
    ("keyboardundo", 0x7A),
    ("keyboardcut", 0x7B),
    ("keyboardcopy", 0x7C),
    ("keyboardpaste", 0x7D),
    ("keyboardfind", 0x7E),
    ("keyboardmute", 0x7F),
    ("keyboardvolumeup", 0x80),
    ("keyboardvolumedown", 0x81),
    ("keyboardlockingcapslock", 0x82),
    ("keyboardlockingnumlock", 0x83),
    ("keyboardlockingscrolllock", 0x84),
    ("keypadcomma", 0x85),
    ("keypadequalsign", 0x86),
    ("keyboardalternateerase", 0x99),
    ("keyboardsysreqattention", 0x9A),
    ("keyboardcancel", 0x9B),
    ("keyboardclear", 0x9C),
    ("keyboardprior", 0x9D),
    ("keyboardreturn", 0x9E),
    ("keyboardseparator", 0x9F),
    ("keyboardout", 0xA0),
    ("keyboardoper", 0xA1),
    ("keyboardclearagain", 0xA2),
    ("keyboardcrselprops", 0xA3),
    ("keyboardexsel", 0xA4),
    ("keypad00", 0xB0),
    ("keypad000", 0xB1),
    ("keypadthousandsseparator", 0xB2),
    ("keypaddecimalseparator", 0xB3),
    ("keypadcurrencyunit", 0xB4),
    ("keypadcurrencysubunit", 0xB5),
    ("keypadleftparen", 0xB6),
    ("keypadrightparen", 0xB7),
    ("keypadleftbrace", 0xB8),
    ("keypadrightbrace", 0xB9),
    ("keypadtab", 0xBA),
    ("keypadbackspace", 0xBB),
    ("keypada", 0xBC),
    ("keypadb", 0xBD),
    ("keypadc", 0xBE),
    ("keypadd", 0xBF),
    ("keypade", 0xC0),
    ("keypadf", 0xC1),
    ("keypadxor", 0xC2),
    ("keypadcaret", 0xC3),
    ("keypadpercent", 0xC4),
    ("keypadless", 0xC5),
    ("keypadgreater", 0xC6),
    ("keypadampersand", 0xC7),
    ("keypaddoubleampersand", 0xC8),
    ("keypadpipe", 0xC9),
    ("keypaddoublepipe", 0xCA),
    ("keypadcolon", 0xCB),
    ("keypadhash", 0xCC),
    ("keypadspace", 0xCD),
    ("keypadat", 0xCE),
    ("keypadexclamation", 0xCF),
    ("keypadmemorystore", 0xD0),
    ("keypadmemoryrecall", 0xD1),
    ("keypadmemoryclear", 0xD2),
    ("keypadmemoryadd", 0xD3),
    ("keypadmemorysubtract", 0xD4),
    ("keypadmemorymultiply", 0xD5),
    ("keypadmemorydivide", 0xD6),
    ("keypadplusminus", 0xD7),
    ("keypadclear", 0xD8),
    ("keypadclearentry", 0xD9),
    ("keypadbinary", 0xDA),
    ("keypadoctal", 0xDB),
    ("keypaddecimal", 0xDC),
    ("keypadhexadecimal", 0xDD),
    ("keyboardleftcontrol", 0xE0),
    ("keyboardleftshift", 0xE1),
    ("keyboardleftalt", 0xE2),
    ("keyboardleftgui", 0xE3),
    ("keyboardrightcontrol", 0xE4),
    ("keyboardrightshift", 0xE5),
    ("keyboardrightalt", 0xE6),
    ("keyboardrightgui", 0xE7),
];

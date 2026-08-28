//! Keystroke → PTY bytes (placeholder for in-house TerminalElement).

/// Map a named key (+ modifiers) to bytes for the PTY.
/// Full parity with Zed's `try_keystroke` will expand here as we leave `gpui-terminal`.
pub fn named_key_to_bytes(key: &str, control: bool, alt: bool) -> Option<Vec<u8>> {
    let key = key.to_ascii_lowercase();

    if control && key.len() == 1 {
        let ch = key.chars().next()?.to_ascii_uppercase();
        if ch.is_ascii_alphabetic() {
            return Some(vec![(ch as u8) - b'@']);
        }
    }

    let mut bytes = match key.as_str() {
        "enter" | "return" => Some(b"\r".to_vec()),
        "backspace" => Some(b"\x7f".to_vec()),
        "tab" => Some(b"\t".to_vec()),
        "escape" => Some(b"\x1b".to_vec()),
        "up" => Some(b"\x1b[A".to_vec()),
        "down" => Some(b"\x1b[B".to_vec()),
        "right" => Some(b"\x1b[C".to_vec()),
        "left" => Some(b"\x1b[D".to_vec()),
        "delete" => Some(b"\x1b[3~".to_vec()),
        _ => None,
    }?;

    if alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

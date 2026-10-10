//! Minimal clipboard support for copying the Antigravity login link.
//!
//! Tries a local clipboard tool first (text passed on stdin, never as an
//! argument), and always also emits an OSC 52 sequence so terminals that
//! support it (including over SSH) receive the text.

use std::io::Write;

const TOOLS: [(&str, &[&str]); 4] = [
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
    ("pbcopy", &[]),
];

/// Encode raw bytes into standard base64 string.
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() {
                T[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// OSC 52 "set clipboard" sequence for `text`.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// Copy `text` to the clipboard. Returns true if a local clipboard tool accepted it.
pub fn copy(text: &str) -> bool {
    let mut out = std::io::stdout();
    let _ = out
        .write_all(osc52(text).as_bytes())
        .and_then(|_| out.flush());
    TOOLS.iter().any(|(cmd, args)| {
        let Ok(mut child) = std::process::Command::new(cmd)
            .args(*args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return false;
        };
        let wrote = child
            .stdin
            .take()
            .is_some_and(|mut s| s.write_all(text.as_bytes()).is_ok());
        wrote && child.wait().map(|s| s.success()).unwrap_or(false)
    })
}

const PASTE_TOOLS: [(&str, &[&str]); 4] = [
    ("wl-paste", &["--no-newline"]),
    ("xclip", &["-selection", "clipboard", "-out"]),
    ("xsel", &["--clipboard", "--output"]),
    ("pbpaste", &[]),
];

/// Paste text from the clipboard using available system tools.
pub fn paste() -> Option<String> {
    for (cmd, args) in &PASTE_TOOLS {
        if let Ok(output) = std::process::Command::new(cmd)
            .args(*args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
        {
            if output.status.success() {
                if let Ok(text) = String::from_utf8(output.stdout) {
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
        }
    }
    None
}

/// Decode standard base64 data into raw bytes.
pub fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for &b in input.as_bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' | b' ' | b'\t' => continue,
            _ => return None,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn osc52_encodes_base64() {
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn base64_roundtrip_decoding() {
        assert_eq!(super::decode_base64("Zg==").as_deref(), Some(b"f".as_slice()));
        assert_eq!(super::decode_base64("Zm8=").as_deref(), Some(b"fo".as_slice()));
        assert_eq!(super::decode_base64("Zm9v").as_deref(), Some(b"foo".as_slice()));
        assert_eq!(super::decode_base64("aGk=").as_deref(), Some(b"hi".as_slice()));
        assert_eq!(
            super::decode_base64("SGVsbG8sIHdvcmxkIQ==").as_deref(),
            Some(b"Hello, world!".as_slice())
        );
        assert_eq!(super::decode_base64("invalid!"), None);
    }
}

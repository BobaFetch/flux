//! The system clipboard behind the `+`/`*` registers.
//!
//! Writes go out as OSC 52 (`ESC ] 52 ; c ; <base64> ST`), which the terminal
//! turns into a system clipboard write: no native dependency, and it works
//! over SSH with a compliant terminal. Reads come from a paste helper probed
//! at startup, because OSC 52 queries are widely disabled and flux has no
//! other paste path.

use std::path::PathBuf;
use std::process::Command;

/// When set to a path, clipboard reads and writes go to that file instead of
/// the system clipboard (deterministic tests; like `FLUX_LSP_CONFIG`).
const FAKE_ENV: &str = "FLUX_CLIPBOARD_FAKE";

/// Paste helpers, first one found on `$PATH` wins: macOS, Wayland, X11 (the
/// X11 tools need their clipboard-selection flags; the default is primary).
/// The Linux entries are confirmed on Linux (see the M7 plan).
const HELPERS: &[(&str, &[&str])] = &[
    ("pbpaste", &[]),
    ("wl-paste", &[]),
    ("xclip", &["-o", "-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--output"]),
];

/// The base64 alphabet (RFC 4648, as OSC 52 carries it).
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut n: u32 = 0;
        for &b in chunk {
            n = (n << 8) | u32::from(b);
        }
        n <<= 8 * (3 - chunk.len());
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The system clipboard as flux sees it.
pub struct Clipboard {
    fake: Option<PathBuf>,
    helper: Option<(String, Vec<String>)>,
}

impl Clipboard {
    /// Probe once at startup: the fake override, else the first paste helper
    /// on `$PATH` (or none, in which case reads miss and only writes go out).
    pub fn probe() -> Self {
        let fake = std::env::var_os(FAKE_ENV).map(PathBuf::from);
        let helper = HELPERS
            .iter()
            .find(|(program, _)| flux_lsp::config::executable(program))
            .map(|(program, args)| {
                (
                    (*program).to_string(),
                    args.iter().map(ToString::to_string).collect(),
                )
            });
        Self { fake, helper }
    }

    /// The current system clipboard text, if it can be read as UTF-8.
    pub fn read(&self) -> Option<String> {
        if let Some(path) = &self.fake {
            return std::fs::read_to_string(path).ok();
        }
        let (program, args) = self.helper.as_ref()?;
        let output = Command::new(program).args(args).output().ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout).ok()
    }

    /// Record a clipboard write for the fake override (real writes go out as
    /// OSC 52, which needs no helper). Returns whether it was recorded.
    pub fn fake_write(&self, text: &str) -> bool {
        if let Some(path) = &self.fake {
            std::fs::write(path, text).is_ok()
        } else {
            false
        }
    }

    /// `text` as an OSC 52 clipboard write (`c` selection).
    pub fn osc52(text: &str) -> String {
        format!("\x1b]52;c;{}\x1b\\", base64_encode(text.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // Multi-byte UTF-8 passes through as bytes.
        assert_eq!(base64_encode("✓".as_bytes()), "4pyT");
    }

    #[test]
    fn osc52_shape() {
        assert_eq!(Clipboard::osc52("hi"), "\x1b]52;c;aGk=\x1b\\");
    }

    #[test]
    fn fake_round_trip() {
        let path = std::env::temp_dir().join(format!("flux-clipboard-test-{}", std::process::id()));
        let clipboard = Clipboard {
            fake: Some(path.clone()),
            helper: None,
        };
        assert!(clipboard.fake_write("hello\n"));
        assert_eq!(clipboard.read().as_deref(), Some("hello\n"));
        let _ = std::fs::remove_file(&path);
    }
}

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use flux_core::{LineEnding, Text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferId(pub usize);

#[derive(Debug)]
pub struct Buffer {
    pub id: BufferId,
    pub text: Text,
    /// The path as the user gave it, which is also how Vim names the buffer.
    pub path: Option<PathBuf>,
    /// The file didn't exist when opened.
    pub new_file: bool,
    /// The file wasn't valid UTF-8 and was loaded with replacement characters. Writing it back
    /// would corrupt it.
    pub invalid_utf8: bool,
}

impl Buffer {
    pub fn scratch(id: BufferId) -> Self {
        Self {
            id,
            text: Text::default(),
            path: None,
            new_file: false,
            invalid_utf8: false,
        }
    }

    /// Load `path`. A missing file gives an empty buffer marked as new, as in Vim.
    pub fn open(id: BufferId, path: &Path) -> io::Result<Self> {
        let mut buffer = Self::scratch(id);
        buffer.path = Some(path.to_path_buf());
        match fs::read(path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(s) => buffer.text = Text::new(&s),
                Err(e) => {
                    buffer.text = Text::new(&String::from_utf8_lossy(e.as_bytes()));
                    buffer.invalid_utf8 = true;
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => buffer.new_file = true,
            Err(e) => return Err(e),
        }
        Ok(buffer)
    }

    /// The name Vim shows for the buffer.
    pub fn name(&self) -> String {
        match &self.path {
            Some(path) => path.display().to_string(),
            None => "[No Name]".into(),
        }
    }

    /// The file-info message Vim shows after loading, e.g. `"main.rs" 42L, 1337B`.
    pub fn file_info(&self) -> String {
        let mut msg = format!("\"{}\"", self.name());
        if self.new_file {
            msg.push_str(" [New]");
            return msg;
        }
        if self.invalid_utf8 {
            msg.push_str(" [invalid UTF-8]");
        }
        if !self.text.has_final_eol() {
            msg.push_str(" [noeol]");
        }
        if self.text.line_ending() == LineEnding::Crlf {
            msg.push_str(" [dos]");
        }
        let lines = if self.text.is_empty() {
            0
        } else {
            self.text.line_count()
        };
        msg.push_str(&format!(" {lines}L, {}B", self.text.len_bytes()));
        msg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_with(contents: &[u8]) -> Buffer {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("flux-view-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("f{}.txt", NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::write(&path, contents).unwrap();
        let mut buf = Buffer::open(BufferId(0), &path).unwrap();
        buf.path = Some("f.txt".into());
        buf
    }

    #[test]
    fn file_info_messages() {
        assert_eq!(buffer_with(b"a\nb\n").file_info(), "\"f.txt\" 2L, 4B");
        assert_eq!(buffer_with(b"").file_info(), "\"f.txt\" 0L, 0B");
        assert_eq!(buffer_with(b"a\nb").file_info(), "\"f.txt\" [noeol] 2L, 3B");
        assert_eq!(buffer_with(b"a\r\n").file_info(), "\"f.txt\" [dos] 1L, 3B");
    }

    #[test]
    fn missing_file_is_new() {
        let buf = Buffer::open(BufferId(0), Path::new("/nonexistent/flux/x.txt")).unwrap();
        assert!(buf.new_file);
        assert_eq!(buf.file_info(), "\"/nonexistent/flux/x.txt\" [New]");
    }

    #[test]
    fn invalid_utf8_is_flagged() {
        let buf = buffer_with(b"ok\xff\n");
        assert!(buf.invalid_utf8);
        assert!(buf.file_info().contains("[invalid UTF-8]"));
    }
}

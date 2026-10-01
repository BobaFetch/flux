//! A server process: messages framed with `Content-Length` headers on its stdin and stdout
//! (the LSP base protocol), its stderr appended to a log file.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

/// Which server a message is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServerId(pub usize);

/// Something a server did.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A message (a response, request or notification).
    Message(ServerId, Value),
    /// The process ended, with what's known about why (`with exit code 1 and signal 0`, as
    /// Neovim words it).
    Exited(ServerId, String),
}

/// A running server.
pub struct Server {
    pub id: ServerId,
    to_server: mpsc::UnboundedSender<Value>,
    kill: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Server {
    /// Start `cmd` in `cwd`; its messages and its exit go to `events`, its stderr to `log`.
    pub fn start(
        id: ServerId,
        cmd: &[String],
        cwd: &Path,
        log: Option<PathBuf>,
        events: mpsc::UnboundedSender<Event>,
    ) -> Result<Self, String> {
        let (program, args) = cmd.split_first().ok_or("empty command")?;
        let mut child = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("{program}: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let (to_server, from_editor) = mpsc::unbounded_channel();
        tokio::spawn(write_messages(stdin, from_editor));
        let (kill, killed) = tokio::sync::oneshot::channel();
        tokio::spawn(supervise(id, child, stdout, events, killed));
        tokio::spawn(log_stderr(program.clone(), stderr, log));
        Ok(Self {
            id,
            to_server,
            kill: Some(kill),
        })
    }

    /// Send a message (dropped if the server has gone).
    pub fn send(&self, message: Value) {
        let _ = self.to_server.send(message);
    }

    /// End the process now (after `shutdown`/`exit`, or when it doesn't answer).
    pub fn kill(&mut self) {
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(());
        }
    }
}

async fn write_messages(
    mut stdin: tokio::process::ChildStdin,
    mut messages: mpsc::UnboundedReceiver<Value>,
) {
    while let Some(message) = messages.recv().await {
        let body = message.to_string();
        let frame = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        if stdin.write_all(frame.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
            break;
        }
    }
}

/// Pass on the server's messages until its stdout ends, then report how the process ended.
async fn supervise(
    id: ServerId,
    mut child: Child,
    stdout: tokio::process::ChildStdout,
    events: mpsc::UnboundedSender<Event>,
    mut killed: tokio::sync::oneshot::Receiver<()>,
) {
    let read = read_messages(id, stdout, &events);
    tokio::pin!(read);
    let error = tokio::select! {
        error = &mut read => error,
        _ = &mut killed => {
            let _ = child.start_kill();
            None
        }
    };
    let why = match child.wait().await {
        _ if error.is_some() => format!("with error: {}", error.unwrap_or_default()),
        Ok(status) => {
            #[cfg(unix)]
            let signal = std::os::unix::process::ExitStatusExt::signal(&status).unwrap_or(0);
            #[cfg(not(unix))]
            let signal = 0;
            let code = status.code().unwrap_or(0);
            format!("with exit code {code} and signal {signal}")
        }
        Err(e) => format!("with error: {e}"),
    };
    let _ = events.send(Event::Exited(id, why));
}

/// Pass on messages until stdout ends (`None`) or something is wrong with it (the error).
async fn read_messages(
    id: ServerId,
    stdout: tokio::process::ChildStdout,
    events: &mpsc::UnboundedSender<Event>,
) -> Option<String> {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_frame(&mut reader).await {
            Ok(Some(body)) => match serde_json::from_slice(&body) {
                Ok(message) => {
                    if events.send(Event::Message(id, message)).is_err() {
                        return None;
                    }
                }
                Err(e) => return Some(format!("invalid message: {e}")),
            },
            Ok(None) => return None,
            Err(e) => return Some(e),
        }
    }
}

/// One message body, or `None` at the end of the stream.
async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Vec<u8>>, String> {
    let mut length = None;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length.ok_or("message without Content-Length")?;
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(body))
}

async fn log_stderr(name: String, stderr: tokio::process::ChildStderr, log: Option<PathBuf>) {
    let mut lines = BufReader::new(stderr).lines();
    let mut file = match log {
        Some(path) => {
            if let Some(dir) = path.parent() {
                let _ = tokio::fs::create_dir_all(dir).await;
            }
            tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await
                .ok()
        }
        None => None,
    };
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(f) = file.as_mut() {
            let _ = f.write_all(format!("[{name}] {line}\n").as_bytes()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_are_read_with_their_length() {
        let data =
            b"Content-Length: 7\r\n\r\n{\"a\":1}Content-Length: 2\r\nContent-Type: x\r\n\r\n{}";
        let mut reader = BufReader::new(&data[..]);
        assert_eq!(
            read_frame(&mut reader).await.unwrap().unwrap(),
            b"{\"a\":1}"
        );
        assert_eq!(read_frame(&mut reader).await.unwrap().unwrap(), b"{}");
        assert_eq!(read_frame(&mut reader).await.unwrap(), None);
    }
}

//! One Radar process per user.
//!
//! The first process listens on a per-user local socket (a named pipe on
//! Windows, a Unix socket in a 0700 per-user directory elsewhere). A later
//! process connects, sends one fixed message and exits: `show` asks the
//! running instance to open/focus its main window, `ping` (used with
//! `--autostart`) only announces itself. The listener accepts nothing but
//! those two exact messages, reads at most [`MAX_MESSAGE`] bytes per
//! connection with a timeout, and never interprets the payload any further.
//!
//! Access: on Windows the pipe's DACL grants access only to SYSTEM and the
//! pipe's owner (the user who started Radar), and the pipe name embeds the
//! user name. On Unix the socket lives in a directory only the user can
//! enter.

use futures::channel::mpsc::UnboundedSender;
use interprocess::local_socket::tokio::{prelude::*, Listener, Stream};
use interprocess::local_socket::{ListenerOptions, Name};
use std::io;
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// Longest accepted message, newline included.
const MAX_MESSAGE: usize = 16;
/// How long a client may take to send its message.
const READ_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    /// Open or focus the main window.
    Show,
    /// Announce only (second launch with `--autostart`).
    Ping,
}

impl Message {
    fn bytes(self) -> &'static [u8] {
        match self {
            Message::Show => b"show\n",
            Message::Ping => b"ping\n",
        }
    }
}

/// Exact-match parse of one client message; anything else is `None`.
pub fn parse_message(buf: &[u8]) -> Option<Message> {
    match buf {
        b"show\n" => Some(Message::Show),
        b"ping\n" => Some(Message::Ping),
        _ => None,
    }
}

pub enum Role {
    /// No other instance: we own the name and must [`serve`] it.
    Primary(Listener),
    /// Another instance is running and got our message; exit.
    Secondary,
}

/// Per-user socket name (`id` distinguishes tests from the real app).
#[derive(Debug, Clone)]
pub struct InstanceName {
    #[cfg(windows)]
    pipe: String,
    #[cfg(not(windows))]
    path: std::path::PathBuf,
}

impl InstanceName {
    pub fn for_user(id: &str) -> Self {
        #[cfg(windows)]
        {
            let user = std::env::var("USERNAME").unwrap_or_default();
            let user: String = user
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            Self {
                pipe: format!("{id}.{user}.instance"),
            }
        }
        #[cfg(not(windows))]
        {
            let base = dirs::runtime_dir()
                .or_else(dirs::data_local_dir)
                .unwrap_or_else(std::env::temp_dir);
            Self {
                path: base.join(id).join("instance.sock"),
            }
        }
    }

    fn name(&self) -> io::Result<Name<'_>> {
        #[cfg(windows)]
        {
            use interprocess::local_socket::GenericNamespaced;
            self.pipe.as_str().to_ns_name::<GenericNamespaced>()
        }
        #[cfg(not(windows))]
        {
            use interprocess::local_socket::GenericFilePath;
            self.path.as_path().to_fs_name::<GenericFilePath>()
        }
    }

    fn listener_options(&self) -> io::Result<ListenerOptions<'_>> {
        let opts = ListenerOptions::new().name(self.name()?);
        #[cfg(windows)]
        let opts = {
            use interprocess::os::windows::local_socket::ListenerOptionsExt as _;
            use interprocess::os::windows::security_descriptor::SecurityDescriptor;
            // Protected DACL: full access for SYSTEM and the owner (OW =
            // Owner Rights) only. Everyone/anonymous get nothing.
            let sddl = widestring::U16CString::from_str("D:P(A;;GA;;;SY)(A;;GA;;;OW)")
                .map_err(io::Error::other)?;
            opts.security_descriptor(SecurityDescriptor::deserialize(&sddl)?)
        };
        #[cfg(not(windows))]
        let opts = {
            use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
            if let Some(dir) = self.path.parent() {
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(dir)?;
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
            }
            // Only reached after a connect failed, so a leftover socket
            // file is a dead instance's corpse.
            opts.try_overwrite(true)
        };
        Ok(opts)
    }
}

/// Becomes the primary instance, or hands `msg` to the running one.
/// Must run inside the tokio runtime.
pub async fn acquire(name: &InstanceName, msg: Message) -> io::Result<Role> {
    let mut last_err = None;
    for _ in 0..3 {
        if let Ok(mut stream) = Stream::connect(name.name()?).await {
            stream.write_all(msg.bytes()).await?;
            stream.flush().await?;
            return Ok(Role::Secondary);
        }
        match name.listener_options()?.create_tokio() {
            Ok(listener) => return Ok(Role::Primary(listener)),
            // Lost a race with another starting instance: connect again.
            Err(e) => last_err = Some(e),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(last_err.unwrap_or_else(|| io::Error::other("single-instance negotiation failed")))
}

/// Accept loop of the primary instance: forwards each valid message.
/// Runs until `tx` is closed.
pub async fn serve(listener: Listener, tx: UnboundedSender<Message>) {
    loop {
        let conn = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("[instance] accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        if tx.is_closed() {
            return;
        }
        let tx = tx.clone();
        tokio::spawn(async move {
            match tokio::time::timeout(READ_TIMEOUT, read_message(conn)).await {
                Ok(Some(msg)) => {
                    let _ = tx.unbounded_send(msg);
                }
                Ok(None) => log::warn!("[instance] ignored unrecognized message"),
                Err(_) => log::warn!("[instance] client timed out"),
            }
        });
    }
}

async fn read_message(mut conn: Stream) -> Option<Message> {
    let mut buf = [0u8; MAX_MESSAGE + 1];
    let mut n = 0;
    while n < buf.len() {
        match conn.read(&mut buf[n..]).await {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(_) => return None,
        }
        if buf[..n].contains(&b'\n') {
            break;
        }
    }
    parse_message(&buf[..n])
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt as _;

    #[test]
    fn parses_only_exact_messages() {
        assert_eq!(parse_message(b"show\n"), Some(Message::Show));
        assert_eq!(parse_message(b"ping\n"), Some(Message::Ping));
        for bad in [
            &b"show"[..],
            b"SHOW\n",
            b"show\nshow\n",
            b" show\n",
            b"open C:\\x\n",
            b"",
            &[0xff; 17],
        ] {
            assert_eq!(parse_message(bad), None, "{bad:?}");
        }
    }

    fn unique_name() -> InstanceName {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        InstanceName::for_user(&format!("com.radar.test.{}.{nanos}", std::process::id()))
    }

    #[test]
    fn second_instance_hands_message_to_first() {
        let rt = radar_core::runtime();
        let name = unique_name();
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        let Role::Primary(listener) = rt.block_on(acquire(&name, Message::Ping)).unwrap() else {
            panic!("first acquire must be primary");
        };
        rt.spawn(serve(listener, tx));

        assert!(matches!(
            rt.block_on(acquire(&name, Message::Show)).unwrap(),
            Role::Secondary
        ));
        let got =
            rt.block_on(async { tokio::time::timeout(Duration::from_secs(5), rx.next()).await });
        assert_eq!(got.unwrap(), Some(Message::Show));

        assert!(matches!(
            rt.block_on(acquire(&name, Message::Ping)).unwrap(),
            Role::Secondary
        ));
        let got =
            rt.block_on(async { tokio::time::timeout(Duration::from_secs(5), rx.next()).await });
        assert_eq!(got.unwrap(), Some(Message::Ping));
    }

    #[test]
    fn garbage_is_not_forwarded() {
        let rt = radar_core::runtime();
        let name = unique_name();
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        let Role::Primary(listener) = rt.block_on(acquire(&name, Message::Ping)).unwrap() else {
            panic!("first acquire must be primary");
        };
        rt.spawn(serve(listener, tx));
        rt.block_on(async {
            let mut s = Stream::connect(name.name().unwrap()).await.unwrap();
            s.write_all(b"calc.exe\n").await.unwrap();
            drop(s);
            let mut s = Stream::connect(name.name().unwrap()).await.unwrap();
            s.write_all(&[b'x'; 64]).await.unwrap();
            drop(s);
            // A valid message afterwards is the first (and only) one seen.
            let mut s = Stream::connect(name.name().unwrap()).await.unwrap();
            s.write_all(b"show\n").await.unwrap();
        });
        let got =
            rt.block_on(async { tokio::time::timeout(Duration::from_secs(5), rx.next()).await });
        assert_eq!(got.unwrap(), Some(Message::Show));
        let none = rt
            .block_on(async { tokio::time::timeout(Duration::from_millis(300), rx.next()).await });
        assert!(none.is_err(), "nothing else may be forwarded");
    }
}

//! Where confirmation mail goes.
//!
//! A world is configured with an outbox directory, a program to run, a relay
//! on loopback, or nothing. With nothing configured, email actions are refused
//! instead of pretending a code was sent. The SMTP sink speaks plain SMTP and
//! never does TLS or login, so the relay on loopback must forward securely.

use std::fmt::Debug;
use std::io::{BufRead, BufReader, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use rand::RngExt as _;

use crate::world::hex_encode;

/// One message to one address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mail {
    /// The recipient, already normalized.
    pub to: String,
    /// A one-line subject.
    pub subject: String,
    /// The plain-text body.
    pub body: String,
}

/// Something that takes a message out of the world.
pub trait MailSink: Debug + Send + Sync {
    /// Hand the message off.
    ///
    /// # Errors
    ///
    /// Fails if the message could not be written or the program did not
    /// finish successfully.
    fn send(&self, mail: &Mail) -> Result<()>;
}

/// Writes each message as a file in a directory, readable only by its owner.
#[derive(Clone, Debug)]
pub struct OutboxSink {
    dir: PathBuf,
}

impl OutboxSink {
    /// An outbox that writes into `dir`, which must already exist.
    #[must_use]
    pub const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl MailSink for OutboxSink {
    fn send(&self, mail: &Mail) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut name = [0_u8; 8];
        rand::rng().fill(&mut name);
        let path = self.dir.join(format!("{}.eml", hex_encode(&name)));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("create {}", path.display()))?;
        write!(
            file,
            "To: {}\nSubject: {}\n\n{}",
            mail.to, mail.subject, mail.body
        )
        .with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }
}

/// Runs a program once per message, with the message in environment
/// variables. No shell is involved, so an address cannot inject a command.
#[derive(Clone, Debug)]
pub struct CommandSink {
    program: PathBuf,
    args: Vec<String>,
}

impl CommandSink {
    /// A sink that runs `program` with `args` for each message.
    #[must_use]
    pub const fn new(program: PathBuf, args: Vec<String>) -> Self {
        Self { program, args }
    }
}

impl MailSink for CommandSink {
    fn send(&self, mail: &Mail) -> Result<()> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .env("ID_MAIL_TO", &mail.to)
            .env("ID_MAIL_SUBJECT", &mail.subject)
            .env("ID_MAIL_BODY", &mail.body)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("run {}", self.program.display()))?;
        let status = child.wait().context("wait for mail program")?;
        if !status.success() {
            bail!("mail program exited with {status}");
        }
        Ok(())
    }
}

const RELAY_TIMEOUT: Duration = Duration::from_secs(10);

/// Hands mail to an SMTP relay on this machine, with plain SMTP and no login.
#[derive(Clone, Debug)]
pub struct SmtpSink {
    relay: SocketAddr,
    from: String,
}

impl SmtpSink {
    /// A sink that sends through `relay`, which must be a loopback address.
    ///
    /// # Errors
    ///
    /// Fails if `relay` is not on loopback or `from` is not a plain address.
    pub fn new(relay: SocketAddr, from: String) -> Result<Self> {
        ensure!(
            relay.ip().is_loopback(),
            "the mail relay must listen on loopback"
        );
        ensure!(
            plain_address(&from),
            "the mail sender is not a plain address"
        );
        Ok(Self { relay, from })
    }
}

impl MailSink for SmtpSink {
    fn send(&self, mail: &Mail) -> Result<()> {
        ensure!(
            plain_address(&mail.to),
            "the recipient is not a plain address"
        );
        ensure!(
            !mail.subject.chars().any(char::is_control),
            "the subject has a line break or control character"
        );
        let stream = TcpStream::connect_timeout(&self.relay, RELAY_TIMEOUT)
            .context("connect to the mail relay")?;
        stream.set_read_timeout(Some(RELAY_TIMEOUT))?;
        stream.set_write_timeout(Some(RELAY_TIMEOUT))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut writer = stream;
        expect(&mut reader, 220)?;
        smtp_command(&mut writer, &mut reader, "EHLO id.local", 250)?;
        smtp_command(
            &mut writer,
            &mut reader,
            &format!("MAIL FROM:<{}>", self.from),
            250,
        )?;
        smtp_command(
            &mut writer,
            &mut reader,
            &format!("RCPT TO:<{}>", mail.to),
            250,
        )?;
        smtp_command(&mut writer, &mut reader, "DATA", 354)?;
        let message = format!(
            "From: {}\r\nTo: {}\r\nSubject: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}.\r\n",
            self.from,
            mail.to,
            mail.subject,
            dot_stuffed(&mail.body)
        );
        writer
            .write_all(message.as_bytes())
            .context("write to the mail relay")?;
        expect(&mut reader, 250)?;
        let _ = smtp_command(&mut writer, &mut reader, "QUIT", 221);
        Ok(())
    }
}

/// A bare address with no spaces, control characters or SMTP syntax in it.
fn plain_address(text: &str) -> bool {
    text.contains('@')
        && text
            .chars()
            .all(|c| c.is_ascii_graphic() && !matches!(c, '<' | '>' | ',' | ';' | '"' | '\\'))
}

/// The body as CRLF lines, with a leading dot doubled so it cannot end the message.
fn dot_stuffed(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 2);
    for line in body.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if line.starts_with('.') {
            out.push('.');
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out
}

fn smtp_command(
    writer: &mut TcpStream,
    reader: &mut impl BufRead,
    line: &str,
    code: u16,
) -> Result<()> {
    writer
        .write_all(format!("{line}\r\n").as_bytes())
        .context("write to the mail relay")?;
    expect(reader, code)
}

/// Read one reply, which may span lines, and check its code.
fn expect(reader: &mut impl BufRead, code: u16) -> Result<()> {
    loop {
        let mut line = String::new();
        ensure!(
            reader
                .read_line(&mut line)
                .context("read from the mail relay")?
                > 0,
            "the mail relay closed the connection"
        );
        let line = line.trim_end();
        ensure!(
            line.get(..3).and_then(|c| c.parse::<u16>().ok()) == Some(code),
            "the mail relay replied: {line}"
        );
        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok(());
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::thread::JoinHandle;

    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let mut salt = [0_u8; 6];
        rand::rng().fill(&mut salt);
        let dir = std::env::temp_dir().join(format!("id-mail-{tag}-{}", hex_encode(&salt)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample() -> Mail {
        Mail {
            to: "a@b.co".to_owned(),
            subject: "Your code".to_owned(),
            body: "Code 123456".to_owned(),
        }
    }

    #[test]
    fn the_outbox_writes_one_private_file_per_message() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("outbox");
        OutboxSink::new(dir.clone()).send(&sample()).unwrap();
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(entries.len(), 1);
        let text = std::fs::read_to_string(&entries[0]).unwrap();
        assert!(text.starts_with("To: a@b.co\nSubject: Your code\n\n"));
        assert!(text.ends_with("Code 123456"));
        let mode = std::fs::metadata(&entries[0]).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_command_sink_passes_the_message_in_the_environment() {
        let dir = scratch("command");
        let out = dir.join("out.txt");
        let script = format!(
            "printf '%s|%s|%s' \"$ID_MAIL_TO\" \"$ID_MAIL_SUBJECT\" \"$ID_MAIL_BODY\" > {}",
            out.display()
        );
        CommandSink::new(PathBuf::from("/bin/sh"), vec!["-c".to_owned(), script])
            .send(&sample())
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "a@b.co|Your code|Code 123456"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failing_command_is_an_error() {
        let sink = CommandSink::new(
            PathBuf::from("/bin/sh"),
            vec!["-c".to_owned(), "exit 3".to_owned()],
        );
        assert!(sink.send(&sample()).is_err());
    }

    /// A relay that records every line it is sent, refusing the recipient if asked.
    fn fake_relay(refuse_recipient: bool) -> (SocketAddr, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            writer.write_all(b"220 fake ready\r\n").unwrap();
            let mut seen = Vec::new();
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let command = line.trim_end().to_owned();
                seen.push(command.clone());
                if command == "DATA" {
                    writer.write_all(b"354 go ahead\r\n").unwrap();
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).unwrap() == 0 {
                            return seen;
                        }
                        let data = line.trim_end().to_owned();
                        seen.push(data.clone());
                        if data == "." {
                            break;
                        }
                    }
                    writer.write_all(b"250 queued\r\n").unwrap();
                    continue;
                }
                let reply: &[u8] = match command.split(' ').next().unwrap_or_default() {
                    "EHLO" => b"250-fake\r\n250 OK\r\n",
                    "RCPT" if refuse_recipient => b"550 no such user\r\n",
                    "QUIT" => {
                        writer.write_all(b"221 bye\r\n").unwrap();
                        break;
                    }
                    _ => b"250 OK\r\n",
                };
                writer.write_all(reply).unwrap();
            }
            seen
        });
        (addr, handle)
    }

    #[test]
    fn the_smtp_sink_speaks_plain_smtp_and_stuffs_dots() {
        let (relay, server) = fake_relay(false);
        let sink = SmtpSink::new(relay, "id@example.test".to_owned()).unwrap();
        let mut mail = sample();
        mail.body = "Code 123456\n.hidden\r\nbye".to_owned();
        sink.send(&mail).unwrap();
        let seen = server.join().unwrap();
        assert_eq!(seen[0], "EHLO id.local");
        assert_eq!(seen[1], "MAIL FROM:<id@example.test>");
        assert_eq!(seen[2], "RCPT TO:<a@b.co>");
        assert_eq!(seen[3], "DATA");
        assert!(seen.contains(&"To: a@b.co".to_owned()));
        assert!(seen.contains(&"Subject: Your code".to_owned()));
        assert!(seen.contains(&"Code 123456".to_owned()));
        assert!(seen.contains(&"..hidden".to_owned()));
        assert!(seen.contains(&"bye".to_owned()));
        assert_eq!(seen.last().map(String::as_str), Some("QUIT"));
    }

    #[test]
    fn a_refused_recipient_is_an_error_and_no_body_is_sent() {
        let (relay, server) = fake_relay(true);
        let sink = SmtpSink::new(relay, "id@example.test".to_owned()).unwrap();
        let error = sink.send(&sample()).unwrap_err();
        assert!(error.to_string().contains("550"), "{error:#}");
        let seen = server.join().unwrap();
        assert!(!seen.contains(&"DATA".to_owned()));
    }

    #[test]
    fn the_smtp_sink_refuses_remote_relays_senders_and_injection_before_connecting() {
        let remote: SocketAddr = "203.0.113.5:25".parse().unwrap();
        let error = SmtpSink::new(remote, "id@example.test".to_owned()).unwrap_err();
        assert!(error.to_string().contains("loopback"));
        let local: SocketAddr = "127.0.0.1:9".parse().unwrap();
        let error = SmtpSink::new(local, "id <x@example.test>".to_owned()).unwrap_err();
        assert!(error.to_string().contains("sender"));

        let sink = SmtpSink::new(local, "id@example.test".to_owned()).unwrap();
        let mut injected = sample();
        injected.subject = "Hi\r\nBcc: x@y.z".to_owned();
        let error = sink.send(&injected).unwrap_err();
        assert!(error.to_string().contains("subject"));
        let mut smuggled = sample();
        smuggled.to = "a@b.co>\r\nRCPT TO:<x@y.z".to_owned();
        let error = sink.send(&smuggled).unwrap_err();
        assert!(error.to_string().contains("recipient"));
    }
}

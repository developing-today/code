//! Where confirmation mail goes.
//!
//! There is no SMTP client here. A world is configured with an outbox
//! directory, a program to run, or nothing. With nothing configured, email
//! actions are refused instead of pretending a code was sent.

use std::fmt::Debug;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
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
}

// Probe: CommandSink send() against a mail program that never exits.
use std::path::PathBuf;
use std::time::Instant;

use id::directory_mail::{CommandSink, Mail, MailSink};

#[test]
fn command_sink_hanging_program() {
    let sink = CommandSink::new(
        PathBuf::from("/bin/sh"),
        vec!["-c".to_owned(), "sleep 30".to_owned()],
    );
    let mail = Mail {
        to: "a@b.co".to_owned(),
        subject: "Your code".to_owned(),
        body: "Code 123456".to_owned(),
    };
    let start = Instant::now();
    // Run send on a thread so the probe itself can report after 3s.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = sink.send(&mail);
        let _ = tx.send(r);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(3)) {
        Ok(r) => println!("send returned after {:?}: {r:?}", start.elapsed()),
        Err(_) => println!(
            "send STILL BLOCKED after {:?} (program sleeps 30s; no timeout in CommandSink)",
            start.elapsed()
        ),
    }
}

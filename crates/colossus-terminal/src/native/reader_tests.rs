use super::*;
use std::{io::Write as _, sync::mpsc};

#[test]
fn cancellation_finishes_a_read_while_an_inherited_writer_remains_open() {
    let (descriptor, mut writer) = filedescriptor::socketpair().unwrap();
    let closed = Arc::new(AtomicBool::new(false));
    let mut reader = Reader {
        descriptor,
        closed: closed.clone(),
        drain: None,
    };
    writer.write_all(b"final output").unwrap();
    let (sender, result) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut output = Vec::new();
        let answer = reader.read_to_end(&mut output).map(|_| output);
        sender.send(answer).unwrap();
    });
    let before_close = result.recv_timeout(Duration::from_millis(150));
    closed.store(true, Ordering::Release);
    let after_close = result.recv_timeout(Duration::from_secs(2));
    // Release the inherited writer before any assertion, including on regression.
    drop(writer);
    thread.join().unwrap();
    assert!(matches!(before_close, Err(mpsc::RecvTimeoutError::Timeout)));
    assert_eq!(after_close.unwrap().unwrap(), b"final output");
}

#[test]
fn cancellation_caps_the_tail_even_when_the_writer_keeps_producing() {
    let (descriptor, mut writer) = filedescriptor::socketpair().unwrap();
    let (ready_sender, ready) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let bytes = [b'x'; 8192];
        writer.write_all(&bytes).unwrap();
        ready_sender.send(()).unwrap();
        for _ in 0..128 {
            if writer.write_all(&bytes).is_err() {
                break;
            }
        }
    });
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    let mut reader = Reader {
        descriptor,
        closed: Arc::new(AtomicBool::new(true)),
        drain: None,
    };
    let mut output = Vec::new();
    let result = reader.read_to_end(&mut output);
    drop(reader);
    thread.join().unwrap();
    result.unwrap();
    assert!(!output.is_empty());
    assert!(output.len() <= FINAL_DRAIN_BYTES);
}

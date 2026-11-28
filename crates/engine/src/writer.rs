use std::io::{self, Write};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

pub type Done = Receiver<io::Result<()>>;

struct Job {
    bytes: Vec<u8>,
    done: Option<Sender<io::Result<()>>>,
}

pub struct PtyWriter {
    jobs: Sender<Job>,
}

pub fn gone() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "the program has exited")
}

pub fn start_writer(name: String, out: Box<dyn Write + Send>) -> io::Result<PtyWriter> {
    let (jobs, rx) = channel();
    thread::Builder::new().name(name).spawn(move || run(out, rx))?;
    Ok(PtyWriter { jobs })
}

fn run(mut out: Box<dyn Write + Send>, rx: Receiver<Job>) {
    let mut broken = false;
    for job in rx {
        let result = if broken {
            Err(gone())
        } else {
            out.write_all(&job.bytes).and_then(|_| out.flush())
        };
        broken = result.is_err();
        if let Some(done) = job.done {
            let _ = done.send(result);
        }
    }
}

pub fn queue(w: &PtyWriter, bytes: Vec<u8>) -> Done {
    let (tx, rx) = channel();
    if let Err(failed) = w.jobs.send(Job { bytes, done: Some(tx) })
        && let Some(done) = failed.0.done
    {
        let _ = done.send(Err(gone()));
    }
    rx
}

pub fn queue_reply(w: &PtyWriter, bytes: Vec<u8>) {
    let _ = w.jobs.send(Job { bytes, done: None });
}

pub fn finish(done: Done) -> io::Result<()> {
    done.recv().unwrap_or_else(|_| Err(gone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct Broken;

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn writes_keep_their_order() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let w = start_writer("w".to_string(), Box::new(Sink(seen.clone()))).unwrap();
        queue_reply(&w, b"a".to_vec());
        let first = queue(&w, b"b".to_vec());
        queue_reply(&w, b"c".to_vec());
        let last = queue(&w, b"d".to_vec());
        finish(first).unwrap();
        finish(last).unwrap();
        assert_eq!(*seen.lock().unwrap(), b"abcd".to_vec());
    }

    #[test]
    fn a_failed_write_fails_the_rest() {
        let w = start_writer("w".to_string(), Box::new(Broken)).unwrap();
        assert!(finish(queue(&w, b"a".to_vec())).is_err());
        assert_eq!(finish(queue(&w, b"b".to_vec())).unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }
}

//! Loopback HTTP fixture for transport tests. One thread serves each connection, so
//! concurrency tests can observe parallel requests; every request is recorded.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub(crate) struct Request {
    pub path: String,
    /// Lowercased raw header block, for asserting what was never sent.
    pub headers: String,
    /// How many earlier requests asked for this same path.
    pub hit: usize,
}

pub(crate) struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    declared: Option<usize>,
    delay: Duration,
}

impl Reply {
    pub(crate) fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self::status(200).body(body)
    }

    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            declared: None,
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub(crate) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Advertise a length different from the body actually sent.
    pub(crate) fn declared(mut self, length: usize) -> Self {
        self.declared = Some(length);
        self
    }

    pub(crate) fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

pub(crate) struct Server {
    base: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| panic!("loopback"));
        listener
            .set_nonblocking(true)
            .unwrap_or_else(|_| panic!("nonblocking"));
        let base = format!(
            "http://{}",
            listener.local_addr().unwrap_or_else(|_| panic!("address"))
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(handler);
        let thread = {
            let (requests, stop) = (requests.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut connections = Vec::new();
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (requests, handler) = (requests.clone(), handler.clone());
                            connections.push(std::thread::spawn(move || {
                                serve(stream, &requests, handler.as_ref());
                            }));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
                for connection in connections {
                    let _ = connection.join();
                }
            })
        };
        Self {
            base,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    pub(crate) fn base(&self) -> &str {
        &self.base
    }

    pub(crate) fn requests(&self) -> Vec<Request> {
        self.requests
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    mut stream: TcpStream,
    requests: &Mutex<Vec<Request>>,
    handler: &(dyn Fn(&Request) -> Reply + Send + Sync),
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    let until = Instant::now() + Duration::from_secs(5);
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
        }
        if bytes.len() > 16 * 1024 || Instant::now() > until {
            return;
        }
    }
    let text = String::from_utf8_lossy(&bytes).to_lowercase();
    let original = String::from_utf8_lossy(&bytes);
    let path = original
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default()
        .to_owned();
    let request = {
        let mut log = requests.lock().unwrap_or_else(|poison| poison.into_inner());
        let hit = log.iter().filter(|r| r.path == path).count();
        let request = Request {
            path,
            headers: text,
            hit,
        };
        log.push(request.clone());
        request
    };
    let reply = handler(&request);
    std::thread::sleep(reply.delay);
    let mut head = format!(
        "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.declared.unwrap_or(reply.body.len())
    );
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
    let _ = stream.flush();
}

//! A replay server as a process (ADR-0059): it answers from fixtures and keeps
//! everything it stores in memory, so it creates no file.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    port: u16,
    home: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// Start a replay server with a home of its own and no `CHARTGEN_DB`, so that the
/// default path of its store is inside that home and nowhere near a real one.
fn replay_server(name: &str) -> Server {
    let home =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/replay");
    let child = Command::new(env!("CARGO_BIN_EXE_chartgen"))
        .args(["--serve", "--port", &port.to_string()])
        .env_clear()
        .env("HOME", &home)
        .env("CHARTGEN_REPLAY_FIXTURES", fixtures)
        .env("CHARTGEN_BIND", "127.0.0.1")
        .current_dir(&home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let server = Server { child, port, home };
    let started = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the server did not start"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    server
}

fn post(port: u16, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        }
        found.push(path);
    }
    found
}

#[test]
fn a_replay_server_writes_no_file() {
    let server = replay_server("replay-no-file");
    // Registering a client is what a live server writes to its store.
    let registered = post(
        server.port,
        "/register",
        r#"{"redirect_uris":["http://127.0.0.1:9/cb"],"client_name":"evaluation"}"#,
    );
    assert!(registered.contains("client_id"), "{registered}");
    assert_eq!(files_under(&server.home), Vec::<PathBuf>::new());
}

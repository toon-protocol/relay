//! The `relay` binary as an operator runs it: its command line, its
//! environment, its startup errors and its shutdown. Each test starts the real
//! binary; what it checks is what the conformance suite checks of the image.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

const SECRET_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";

/// A port nothing listens on at the moment of asking.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("the OS gives out a port")
        .local_addr()
        .expect("a bound listener has an address")
        .port()
}

fn command(data: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_relay"));
    command
        .env_clear()
        .env("TOON_SECRET_KEY", SECRET_KEY)
        .env("TOON_DATA_DIR", data.path())
        .stdin(Stdio::null());
    command
}

fn ports(command: &mut Command) -> (u16, u16) {
    let (write, read) = (free_port(), free_port());
    command
        .env("TOON_WRITE_HOST", "127.0.0.1")
        .env("TOON_HOST", "127.0.0.1")
        .env("TOON_BLS_PORT", write.to_string())
        .env("TOON_RELAY_PORT", read.to_string());
    (write, read)
}

fn refused(command: &mut Command) -> String {
    let output = command.output().expect("the binary runs");
    assert!(
        !output.status.success(),
        "the relay should not have started"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(stderr.contains("Error: "), "no Error: line in {stderr:?}");
    stderr
}

/// A plain `GET` on `127.0.0.1:port`: the status line and the body.
fn get(port: u16, path: &str) -> Option<(String, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    let (head, body) = response.split_once("\r\n\r\n")?;
    Some((head.lines().next()?.to_string(), body.to_string()))
}

/// A started relay, killed if a test ends before it does.
struct Running(Option<Child>);

impl Running {
    fn child(&self) -> &Child {
        self.0.as_ref().expect("the relay has not been waited on")
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn start(command: &mut Command, write_port: u16) -> Running {
    let child = Running(Some(
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the binary starts"),
    ));
    let deadline = Instant::now() + Duration::from_secs(20);
    while get(write_port, "/health").is_none() {
        assert!(Instant::now() < deadline, "the relay never answered");
        std::thread::sleep(Duration::from_millis(50));
    }
    child
}

fn signal(child: &Running, name: &str) {
    let status = Command::new("kill")
        .arg(format!("-{name}"))
        .arg(child.child().id().to_string())
        .status()
        .expect("kill runs");
    assert!(status.success());
}

fn exit_within(mut running: Running, seconds: u64) -> Output {
    let mut child = running.0.take().expect("the relay has not been waited on");
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while child.try_wait().expect("the child can be polled").is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the relay did not exit");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    child
        .wait_with_output()
        .expect("the output of an exited child")
}

#[test]
fn an_unknown_flag_is_an_error() {
    let data = tempfile::tempdir().expect("a temp dir");
    let stderr = refused(command(&data).arg("--no-such-flag"));
    assert!(stderr.contains("--no-such-flag"));
}

#[test]
fn a_bare_argument_and_a_flag_without_its_value_are_errors() {
    let data = tempfile::tempdir().expect("a temp dir");
    refused(command(&data).arg("stray"));
    refused(command(&data).arg("--relay-port"));
}

#[test]
fn dev_mode_true_is_refused_by_name_whether_set_or_flagged() {
    let data = tempfile::tempdir().expect("a temp dir");
    let stderr = refused(command(&data).env("TOON_DEV_MODE", "true"));
    assert!(stderr.contains("TOON_DEV_MODE"));
    refused(command(&data).arg("--dev-mode"));
}

#[test]
fn dev_mode_false_or_unset_starts_normally() {
    for dev_mode in [Some("false"), None] {
        let data = tempfile::tempdir().expect("a temp dir");
        let mut command = command(&data);
        if let Some(value) = dev_mode {
            command.env("TOON_DEV_MODE", value);
        }
        let (write, _) = ports(&mut command);
        let child = start(&mut command, write);
        signal(&child, "TERM");
        assert!(exit_within(child, 10).status.success());
    }
}

#[test]
fn verify_workers_is_logged_once_as_having_no_effect_and_ignored() {
    let data = tempfile::tempdir().expect("a temp dir");
    let mut command = command(&data);
    command.env("TOON_VERIFY_WORKERS", "4");
    let (write, _) = ports(&mut command);
    let child = start(&mut command, write);
    signal(&child, "TERM");
    let output = exit_within(child, 10);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("has no effect").count(), 1, "{stdout}");
}

#[test]
fn unknown_environment_variables_are_ignored() {
    let data = tempfile::tempdir().expect("a temp dir");
    let mut command = command(&data);
    command
        .env("TOON_NO_SUCH_SETTING", "1")
        .env("SOMETHING_ELSE", "x");
    let (write, _) = ports(&mut command);
    let child = start(&mut command, write);
    signal(&child, "TERM");
    assert!(exit_within(child, 10).status.success());
}

#[test]
fn a_flag_beats_its_variable() {
    let data = tempfile::tempdir().expect("a temp dir");
    let mut command = command(&data);
    let (_, read) = ports(&mut command);
    let flagged = free_port();
    command
        .env("TOON_BLS_PORT", "1")
        .arg("--bls-port")
        .arg(flagged.to_string());
    // The variable alone would be a port nothing can listen on here.
    let child = start(&mut command, flagged);
    assert!(get(read, "/").is_some());
    signal(&child, "TERM");
    exit_within(child, 10);
}

#[test]
fn metrics_has_the_verify_block_and_the_lane_bounds_and_no_event_loop() {
    let data = tempfile::tempdir().expect("a temp dir");
    let mut command = command(&data);
    let (write, _) = ports(&mut command);
    let child = start(&mut command, write);
    let (status, body) = get(write, "/metrics").expect("the relay answers");
    assert!(status.contains("200"), "{status}");
    let metrics: serde_json::Value = serde_json::from_str(&body).expect("a JSON body");
    let mut keys: Vec<_> = metrics.as_object().expect("an object").keys().collect();
    keys.sort();
    assert_eq!(keys, ["ephemeralWriteLane", "timestamp", "verify"]);
    assert_eq!(metrics["verify"]["implementation"], "libsecp256k1-native");
    assert_eq!(metrics["verify"]["workers"], 0);
    assert_eq!(metrics["verify"]["count"], 0);
    assert_eq!(
        metrics["ephemeralWriteLane"],
        serde_json::json!({
            "enabled": true,
            "rateLimit": { "maxRequests": 200, "windowMs": 10_000 },
            "maxBodyBytes": 8192
        })
    );
    drop(child);
}

#[test]
fn sigterm_and_sigint_stop_the_relay_cleanly_even_with_a_client_connected() {
    for name in ["TERM", "INT"] {
        let data = tempfile::tempdir().expect("a temp dir");
        let mut command = command(&data);
        let (write, read) = ports(&mut command);
        let child = start(&mut command, write);

        // A client holding a WebSocket open must not hold the relay up.
        let mut socket = TcpStream::connect(("127.0.0.1", read)).expect("the read port accepts");
        write!(
            socket,
            "GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .expect("the handshake is sent");
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("a read timeout");
        let mut handshake = [0_u8; 12];
        socket
            .read_exact(&mut handshake)
            .expect("the relay answers");
        assert!(handshake.starts_with(b"HTTP/1.1 101"));

        signal(&child, name);
        let output = exit_within(child, 10);
        assert!(output.status.success(), "SIG{name}: {:?}", output.status);
        drop(socket);
    }
}

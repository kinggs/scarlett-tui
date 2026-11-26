use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

#[test]
fn daemon_handles_fragmented_stdin_and_stale_set_rejection() {
    let bin = env!("CARGO_BIN_EXE_scarlett-tui");
    let mut child = Command::new(bin)
        .args(["--daemon", "--card", "999"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn scarlett-tui --daemon");

    let mut stdin = child.stdin.take().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);

    // First line: initial disconnected state snapshot
    let mut line = String::new();
    let n = reader.read_line(&mut line).expect("read initial state");
    assert!(n > 0, "expected initial state line");
    let v: serde_json::Value = serde_json::from_str(&line).expect("valid json");
    assert_eq!(v["type"], "state");
    assert_eq!(v["connected"], false);

    // Send a fragmented `refresh\n` command across separate pipe writes.
    stdin.write_all(b"ref").expect("write part 1");
    stdin.flush().expect("flush part 1");
    thread::sleep(Duration::from_millis(80));
    stdin.write_all(b"resh\n").expect("write part 2");
    stdin.flush().expect("flush part 2");

    line.clear();
    let n = reader.read_line(&mut line).expect("read refresh response");
    assert!(n > 0, "expected response to fragmented refresh");
    let v: serde_json::Value = serde_json::from_str(&line).expect("valid json");
    assert_eq!(v["type"], "state");

    // Send a `set` with a bogus generation -> should emit an error payload cleanly.
    stdin
        .write_all(b"set 1 1 99999\n")
        .expect("write stale set");
    stdin.flush().expect("flush stale set");

    line.clear();
    let n = reader.read_line(&mut line).expect("read error response");
    assert!(n > 0, "expected error response");
    let v: serde_json::Value = serde_json::from_str(&line).expect("valid json");
    assert_eq!(v["type"], "error");

    // Close stdin -> daemon must exit cleanly with status 0.
    drop(stdin);
    let status = child.wait().expect("wait for daemon exit");
    assert!(status.success(), "daemon should exit 0 on EOF: {status}");
}

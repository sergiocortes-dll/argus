use std::sync::mpsc;
use std::time::{Duration, Instant};

use terminal_infra::{Pty, PtyEvent, PtyGrid, ShellCommand};

const MARKER: &str = "argus-pty-ok";
const TIMEOUT: Duration = Duration::from_secs(15);
const CURSOR_POSITION_REQUEST: &[u8] = b"\x1b[6n";

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn echo_command() -> ShellCommand {
    if cfg!(windows) {
        ShellCommand::new("cmd.exe")
            .arg("/c")
            .arg(format!("echo {MARKER}"))
    } else {
        ShellCommand::new("/bin/sh")
            .arg("-c")
            .arg(format!("echo {MARKER}"))
    }
}

#[test]
fn spawned_command_output_reaches_the_callback() {
    let (tx, rx) = mpsc::channel();
    let mut pty = Pty::spawn(
        echo_command(),
        PtyGrid { cols: 80, rows: 24 },
        move |event| {
            let _ = tx.send(event);
        },
    )
    .expect("spawn pty");

    let deadline = Instant::now() + TIMEOUT;
    let mut output = Vec::new();
    let mut exited = None;
    let mut cursor_reported = false;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(PtyEvent::Output(bytes)) => {
                output.extend(bytes);
                // ConPTY waits for a cursor position report before emitting output.
                if !cursor_reported && contains(&output, CURSOR_POSITION_REQUEST) {
                    pty.write(b"\x1b[1;1R").expect("answer cursor position");
                    cursor_reported = true;
                }
            }
            Ok(PtyEvent::Exited(code)) => exited = Some(code),
            Err(_) => {}
        }
        if exited.is_some() && String::from_utf8_lossy(&output).contains(MARKER) {
            break;
        }
    }

    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains(MARKER),
        "marker not found in output: {text:?}"
    );
    assert_eq!(exited, Some(Some(0)), "child did not exit cleanly");
}

#[test]
fn default_shell_is_resolved() {
    let shell = ShellCommand::default_shell();
    assert!(!shell.program.is_empty());
}

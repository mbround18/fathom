//! The built helper binary against a temp root (T015): handshake, `Stat` of the root, exit 0 on
//! stdin EOF and 70 on a protocol error. Debug builds only: release builds ignore
//! `DEPTHS_HELPER_ROOT` and always open `/volume`.

#![cfg(debug_assertions)]

use std::io::{Read, Write};
use std::process::{Child, ChildStdout, Command, Stdio};

use depths_volumes::frame::{FrameKind, read_frame, write_frame, write_json};
use depths_volumes::mount::EntryKind;
use depths_volumes::op::{Hello, HelperError, Op, PROTO, Reply, parse_answer};
use depths_volumes::path::RelPath;

fn spawn(root: &std::path::Path) -> (Child, ChildStdout, Hello) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_depths-volume-helper"))
        .env("DEPTHS_HELPER_ROOT", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let hello = read_frame(&mut stdout)
        .unwrap()
        .unwrap()
        .parse(FrameKind::Hello)
        .unwrap();
    (child, stdout, hello)
}

#[test]
fn handshake_then_stat_of_the_root_then_exit_0_on_eof() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    let (mut child, mut stdout, hello) = spawn(dir.path());
    assert_eq!(
        hello,
        Hello {
            proto: PROTO,
            openat2: true,
            uid: rustix::process::getuid().as_raw(),
            gid: rustix::process::getgid().as_raw(),
        }
    );

    let mut stdin = child.stdin.take().unwrap();
    let stat = |path: &str, hash| Op::Stat {
        path: RelPath::parse(path).unwrap(),
        raw_name: None,
        hash,
    };
    write_json(&mut stdin, FrameKind::Op, &stat("", false)).unwrap();
    write_json(&mut stdin, FrameKind::Op, &stat("a.txt", true)).unwrap();
    write_json(&mut stdin, FrameKind::Op, &stat("missing", false)).unwrap();
    stdin.flush().unwrap();

    let answer = |out: &mut ChildStdout| parse_answer(&read_frame(out).unwrap().unwrap()).unwrap();
    let Ok(Reply::Stat(root)) = answer(&mut stdout) else {
        panic!("no Stat reply for the root");
    };
    assert_eq!(root.kind, EntryKind::Dir);
    let Ok(Reply::Stat(file)) = answer(&mut stdout) else {
        panic!("no Stat reply for a.txt");
    };
    assert_eq!(file.kind, EntryKind::File);
    assert_eq!(file.size, 5);
    assert_eq!(file.fingerprint.unwrap().size(), 5);
    assert_eq!(answer(&mut stdout), Err(HelperError::NotFound));

    drop(stdin);
    let mut rest = Vec::new();
    stdout.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty(), "no output after the answers");
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

#[test]
fn ops_it_does_not_implement_yet_are_unsupported_and_their_data_is_drained() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut stdout, _) = spawn(dir.path());
    let mut stdin = child.stdin.take().unwrap();
    let untar = Op::Untar {
        dir: RelPath::root(),
    };
    write_json(&mut stdin, FrameKind::Op, &untar).unwrap();
    write_frame(&mut stdin, FrameKind::Data, b"tar bytes").unwrap();
    write_frame(&mut stdin, FrameKind::End, b"").unwrap();
    let root = Op::Stat {
        path: RelPath::root(),
        raw_name: None,
        hash: false,
    };
    write_json(&mut stdin, FrameKind::Op, &root).unwrap();
    drop(stdin);

    let first = parse_answer(&read_frame(&mut stdout).unwrap().unwrap()).unwrap();
    assert_eq!(first, Err(HelperError::Unsupported));
    let second = parse_answer(&read_frame(&mut stdout).unwrap().unwrap()).unwrap();
    assert!(matches!(second, Ok(Reply::Stat(_))));
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

/// Sends `bytes` after the handshake and expects a `protocol` Error frame, then exit 70.
fn assert_protocol_error(bytes: &[u8]) {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut stdout, _) = spawn(dir.path());
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(bytes).unwrap();
    drop(stdin);
    let frame = read_frame(&mut stdout).unwrap().unwrap();
    assert_eq!(frame.kind, FrameKind::Error);
    let error: HelperError = frame.parse(FrameKind::Error).unwrap();
    assert!(matches!(error, HelperError::Protocol { .. }), "{error:?}");
    assert!(read_frame(&mut stdout).unwrap().is_none());
    assert_eq!(child.wait().unwrap().code(), Some(70));
}

#[test]
fn an_unknown_frame_kind_exits_70() {
    assert_protocol_error(&[0, 0, 0, 0, 9]);
}

#[test]
fn a_frame_that_is_not_an_op_exits_70() {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, FrameKind::Data, b"stray").unwrap();
    assert_protocol_error(&bytes);
}

#[test]
fn an_op_with_an_invalid_path_exits_70() {
    let mut bytes = Vec::new();
    write_frame(
        &mut bytes,
        FrameKind::Op,
        br#"{"op":"stat","path":"../outside"}"#,
    )
    .unwrap();
    assert_protocol_error(&bytes);
}

#[test]
fn a_truncated_frame_exits_70() {
    assert_protocol_error(&[0, 0, 0, 10, 1, b'{']);
}

#[test]
fn a_frame_over_the_limit_exits_70() {
    assert_protocol_error(&[0x00, 0x10, 0x00, 0x01, 1]);
}

//! Drives `serve` in-process: a script of Op (and Data… End) frames in, the answers out.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::Cursor;
use std::path::Path;

use depths_volume_helper::resolve::Root;
use depths_volume_helper::{EXIT_OK, serve};
use depths_volumes::frame::{Frame, FrameKind, read_frame, write_data, write_frame, write_json};
use depths_volumes::op::{HelperError, Op, Reply, parse_answer};
use depths_volumes::path::RelPath;

pub fn rp(path: &str) -> RelPath {
    RelPath::parse(path).unwrap()
}

/// The frames the server sends, in order.
#[derive(Default)]
pub struct Script {
    input: Vec<u8>,
}

impl Script {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn op(mut self, op: &Op) -> Self {
        write_json(&mut self.input, FrameKind::Op, op).unwrap();
        self
    }

    /// `bytes` as Data frames, then End.
    pub fn data(mut self, bytes: &[u8]) -> Self {
        write_data(&mut self.input, bytes).unwrap();
        write_frame(&mut self.input, FrameKind::End, &[]).unwrap();
        self
    }

    /// Raw bytes, for a stream that is cut or malformed.
    pub fn raw(mut self, bytes: &[u8]) -> Self {
        self.input.extend_from_slice(bytes);
        self
    }

    /// Runs `serve` on `root` until the script ends; the Hello frame is checked and dropped.
    pub fn run(self, root: &Path) -> Answers {
        let root = Root::open(root).unwrap();
        let mut output = Vec::new();
        let code = serve(root.into_fd(), Cursor::new(self.input), &mut output);
        let mut out = Cursor::new(output);
        let mut frames = VecDeque::new();
        while let Some(frame) = read_frame(&mut out).unwrap() {
            frames.push_back(frame);
        }
        assert_eq!(frames.pop_front().unwrap().kind, FrameKind::Hello);
        Answers { frames, code }
    }
}

/// What `serve` wrote after Hello, and its exit code.
pub struct Answers {
    frames: VecDeque<Frame>,
    pub code: i32,
}

impl Answers {
    fn frame(&mut self) -> Frame {
        self.frames.pop_front().expect("another frame")
    }

    /// The next Reply or Error.
    pub fn answer(&mut self) -> Result<Reply, HelperError> {
        parse_answer(&self.frame()).unwrap()
    }

    /// The next Data… End, joined. An Error frame in place of End is returned as `Err`.
    pub fn data(&mut self) -> Result<Vec<u8>, HelperError> {
        let mut bytes = Vec::new();
        loop {
            let frame = self.frame();
            match frame.kind {
                FrameKind::Data => bytes.extend_from_slice(&frame.body),
                FrameKind::End => return Ok(bytes),
                FrameKind::Error => return Err(frame.parse(FrameKind::Error).unwrap()),
                other => panic!("unexpected {other:?} frame in a data stream"),
            }
        }
    }

    /// No frames are left and the loop ended on EOF.
    pub fn done(&self) {
        assert!(self.frames.is_empty(), "{} frames left", self.frames.len());
        assert_eq!(self.code, EXIT_OK);
    }
}

/// Runs a single op (with `data` as Data… End when given) and returns its answer.
pub fn one(root: &Path, op: &Op, data: Option<&[u8]>) -> (Result<Reply, HelperError>, Answers) {
    let mut script = Script::new().op(op);
    if let Some(data) = data {
        script = script.data(data);
    }
    let mut answers = script.run(root);
    (answers.answer(), answers)
}

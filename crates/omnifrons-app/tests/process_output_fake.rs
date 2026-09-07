//! Exercises the `ProcessOutput` port against an in-memory fake: seeded
//! frames arrive in `seq` order with `dropped_before` preserved, and a
//! second `subscribe` for the same id is rejected rather than silently
//! handing back a second, independent receiver.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use omnifrons_app::{
    FramePayload, OutputFrame, OutputStream, ProcessId, ProcessOutput, SupervisorError,
};

/// A `ProcessOutput` test double: frames are seeded ahead of time per id,
/// then drained into a `sync_channel` on `subscribe`, exactly once per id.
struct FakeProcessOutput {
    seeded: HashMap<ProcessId, Vec<OutputFrame>>,
    subscribed: HashSet<ProcessId>,
}

impl FakeProcessOutput {
    fn new() -> Self {
        Self {
            seeded: HashMap::new(),
            subscribed: HashSet::new(),
        }
    }

    fn seed(&mut self, id: ProcessId, frames: Vec<OutputFrame>) {
        self.seeded.insert(id, frames);
    }
}

impl ProcessOutput for FakeProcessOutput {
    fn subscribe(&mut self, id: ProcessId) -> Result<Receiver<OutputFrame>, SupervisorError> {
        if self.subscribed.contains(&id) {
            return Err(SupervisorError::AlreadySubscribed);
        }
        let frames = self
            .seeded
            .remove(&id)
            .ok_or(SupervisorError::UnknownProcess)?;
        self.subscribed.insert(id);

        // Bounded, like the real capture path's per-child queue -- large
        // enough that seeding a handful of test frames never blocks.
        let (tx, rx): (SyncSender<OutputFrame>, Receiver<OutputFrame>) = sync_channel(64);
        for frame in frames {
            tx.try_send(frame)
                .expect("test seed count must fit the channel capacity");
        }
        Ok(rx)
    }
}

fn text(seq: u64, stream: OutputStream, text: &str) -> OutputFrame {
    OutputFrame::new(
        seq,
        FramePayload::Text {
            stream,
            text: text.to_string(),
            continued: false,
        },
    )
}

#[test]
fn seeded_frames_arrive_in_seq_order() {
    let id = ProcessId(1);
    let mut fake = FakeProcessOutput::new();
    fake.seed(
        id,
        vec![
            text(0, OutputStream::Stdout, "line 1 out"),
            text(1, OutputStream::Stdout, "line 2 out"),
            text(2, OutputStream::Stderr, "line 5 err"),
        ],
    );

    let rx = fake.subscribe(id).expect("first subscribe must succeed");
    let received: Vec<OutputFrame> = rx.iter().collect();

    assert_eq!(received.len(), 3);
    assert_eq!(received[0].seq, 0);
    assert_eq!(received[1].seq, 1);
    assert_eq!(received[2].seq, 2);
    assert_eq!(
        received[2].payload,
        FramePayload::Text {
            stream: OutputStream::Stderr,
            text: "line 5 err".to_string(),
            continued: false,
        }
    );
}

#[test]
fn dropped_before_is_preserved_end_to_end() {
    let id = ProcessId(2);
    let mut fake = FakeProcessOutput::new();
    fake.seed(
        id,
        vec![OutputFrame::with_dropped_before(
            10,
            3,
            FramePayload::Text {
                stream: OutputStream::Stdout,
                text: "line 11 out".to_string(),
                continued: false,
            },
        )],
    );

    let rx = fake.subscribe(id).expect("subscribe must succeed");
    let frame = rx.recv().expect("the seeded frame must be delivered");

    assert_eq!(frame.dropped_before, 3);
}

#[test]
fn second_subscribe_for_the_same_id_is_rejected() {
    let id = ProcessId(3);
    let mut fake = FakeProcessOutput::new();
    fake.seed(id, vec![text(0, OutputStream::Stdout, "line 1 out")]);

    let _first = fake.subscribe(id).expect("first subscribe must succeed");
    let second = fake
        .subscribe(id)
        .expect_err("second subscribe for the same id must be rejected");

    assert_eq!(second, SupervisorError::AlreadySubscribed);
}

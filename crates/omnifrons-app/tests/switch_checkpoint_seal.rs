//! Checkpoint integrity (target-architecture.md § Model and harness
//! switching): a sealed checkpoint opens only when its body still digests
//! to the value its `integrity` record claims.
//!
//! Only compiled with `--features contract-tests`: the fake hasher lives
//! behind that feature.

#![cfg(feature = "contract-tests")]

use omnifrons_app::content_hasher::ContentHasher;
use omnifrons_app::contract::publication::FakeHasher;
use omnifrons_app::switch_checkpoint::{OpenError, open, seal};
use omnifrons_domain::adapter::AdapterId;
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::RunId;
use omnifrons_domain::scope::ProcessTerminalState;
use omnifrons_domain::switch_checkpoint::{
    CheckpointError, CheckpointFacts, Inventory, OutgoingRun, SwitchCheckpoint,
};

fn checkpoint() -> SwitchCheckpoint {
    SwitchCheckpoint::new(CheckpointFacts {
        created_unix_seconds: 1_795_770_799,
        project: Sha256Digest([0x5a; 32]),
        outgoing: OutgoingRun {
            adapter: AdapterId::claude_code(),
            run: Some(RunId::new("run-1").expect("run id")),
            terminal: Some(ProcessTerminalState::Exited { code: Some(0) }),
        },
        target: AdapterId::stream_json_cli(),
        goal: None,
        definition: None,
        inventory: Inventory::new(Vec::new(), true).expect("inventory"),
    })
    .expect("checkpoint")
}

#[test]
fn a_sealed_checkpoint_opens_to_itself() {
    let sealed = seal(&checkpoint(), &FakeHasher);
    let body = checkpoint().encode_body();
    let digest = FakeHasher.sha256(body.as_bytes());
    assert_eq!(
        sealed,
        format!("{body}integrity=71:sha256:{}\n", digest.to_hex())
    );
    assert_eq!(open(&sealed, &FakeHasher), Ok(checkpoint()));
}

#[test]
fn a_changed_body_fails_the_integrity_check() {
    let sealed = seal(&checkpoint(), &FakeHasher);
    let tampered = sealed.replacen("created=10:1795770799", "created=10:1795770800", 1);
    assert_eq!(
        open(&tampered, &FakeHasher),
        Err(OpenError::IntegrityMismatch)
    );
}

#[test]
fn an_unsealed_or_malformed_checkpoint_is_refused() {
    let body = checkpoint().encode_body();
    assert_eq!(
        open(&body, &FakeHasher),
        Err(OpenError::Malformed(CheckpointError::MissingField(
            "integrity"
        )))
    );
    let newer = body.replacen("checkpoint/1.0", "checkpoint/2.0", 1);
    let digest = FakeHasher.sha256(newer.as_bytes());
    let sealed_newer = format!("{newer}integrity=71:sha256:{}\n", digest.to_hex());
    assert_eq!(
        open(&sealed_newer, &FakeHasher),
        Err(OpenError::Malformed(CheckpointError::UnsupportedMajor(2)))
    );
}

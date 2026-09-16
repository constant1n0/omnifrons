//! Non-product evidence-record tooling for VP-001 (design.md D2): parses the
//! append-only `docs/evidence/VP-001/{baselines,records}.md` record grammar,
//! validates it against the closed V1-V8 admission contract, and derives one
//! honest VP-S6 outcome as a pure function. No product crate depends on this
//! crate (`cargo test --workspace` runs it, but it never links into the
//! shipped binary).

pub mod derive;
pub mod record;
pub mod validate;

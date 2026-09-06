//! The catalog of demo harness kinds a caller may request, and the
//! validated request shape.
//!
//! No program path or argument ever crosses IPC (see
//! `docs/spike-log.md` § IPC contract): the renderer names a closed
//! [`HarnessKind`] plus small bounded numbers, never a command line. The
//! supervisor turns a validated [`HarnessRequest`] into a real
//! `ProcessSpec` internally, using a launcher path supplied at construction
//! time (`TokioProcessSupervisor::with_demo_launcher`), not one carried by
//! the request.

/// The closed set of demo harness behaviors a caller may request.
///
/// Closed and exhaustive by design: the renderer names one of these kinds,
/// never a program path or argument vector (`docs/spike-log.md` § IPC
/// contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HarnessKind {
    /// Emit `lines` lines at `rate_hz`, then exit `0`.
    DemoLines,
    /// Behave like [`Self::DemoLines`], but ignore `SIGTERM` so a caller
    /// must escalate to a forceful stop to terminate it.
    DemoIgnoresSigterm,
}

/// Why a [`HarnessRequest`] was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvalidRequest {
    /// `rate_hz` was `0`; a harness must emit at a positive rate.
    RateHzZero,
    /// `rate_hz` exceeded the bound (`1000`).
    RateHzTooHigh,
    /// `lines` was `0`; a harness must emit at least one line.
    LinesZero,
    /// `lines` exceeded the bound (`100_000`).
    LinesTooHigh,
}

impl std::fmt::Display for InvalidRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::RateHzZero => "rate_hz must not be zero",
            Self::RateHzTooHigh => "rate_hz must not exceed 1000",
            Self::LinesZero => "lines must not be zero",
            Self::LinesTooHigh => "lines must not exceed 100_000",
        };
        f.write_str(message)
    }
}

impl std::error::Error for InvalidRequest {}

/// The upper bound on [`HarnessRequest::rate_hz`], inclusive.
const MAX_RATE_HZ: u16 = 1000;
/// The upper bound on [`HarnessRequest::lines`], inclusive.
const MAX_LINES: u32 = 100_000;

/// A validated request to run a demo harness: which kind, at what rate, for
/// how many lines.
///
/// Constructible only through [`Self::new`], so every live `HarnessRequest`
/// has already passed the bounds every caller (the shell's IPC commands,
/// the supervisor's own tests) must otherwise re-check by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HarnessRequest {
    kind: HarnessKind,
    rate_hz: u16,
    lines: u32,
}

impl HarnessRequest {
    /// Validate and build a request.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRequest`] if `rate_hz` is `0` or exceeds `1000`, or
    /// if `lines` is `0` or exceeds `100_000`.
    pub fn new(kind: HarnessKind, rate_hz: u16, lines: u32) -> Result<Self, InvalidRequest> {
        if rate_hz == 0 {
            return Err(InvalidRequest::RateHzZero);
        }
        if rate_hz > MAX_RATE_HZ {
            return Err(InvalidRequest::RateHzTooHigh);
        }
        if lines == 0 {
            return Err(InvalidRequest::LinesZero);
        }
        if lines > MAX_LINES {
            return Err(InvalidRequest::LinesTooHigh);
        }
        Ok(Self {
            kind,
            rate_hz,
            lines,
        })
    }

    /// The requested harness kind.
    #[must_use]
    pub const fn kind(&self) -> HarnessKind {
        self.kind
    }

    /// The validated emission rate, in Hz (`1..=1000`).
    #[must_use]
    pub const fn rate_hz(&self) -> u16 {
        self.rate_hz
    }

    /// The validated line count (`1..=100_000`).
    #[must_use]
    pub const fn lines(&self) -> u32 {
        self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::{HarnessKind, HarnessRequest, InvalidRequest};

    #[test]
    fn rejects_zero_rate_hz() {
        let error = HarnessRequest::new(HarnessKind::DemoLines, 0, 10).unwrap_err();
        assert_eq!(error, InvalidRequest::RateHzZero);
    }

    #[test]
    fn rejects_rate_hz_above_1000() {
        let error = HarnessRequest::new(HarnessKind::DemoLines, 1001, 10).unwrap_err();
        assert_eq!(error, InvalidRequest::RateHzTooHigh);
    }

    #[test]
    fn rejects_zero_lines() {
        let error = HarnessRequest::new(HarnessKind::DemoLines, 10, 0).unwrap_err();
        assert_eq!(error, InvalidRequest::LinesZero);
    }

    #[test]
    fn rejects_lines_above_100_000() {
        let error = HarnessRequest::new(HarnessKind::DemoLines, 10, 100_001).unwrap_err();
        assert_eq!(error, InvalidRequest::LinesTooHigh);
    }

    #[test]
    fn accepts_a_well_formed_request() {
        let request = HarnessRequest::new(HarnessKind::DemoIgnoresSigterm, 1000, 100_000)
            .expect("boundary values (1000 Hz, 100_000 lines) must be accepted");
        assert_eq!(request.kind(), HarnessKind::DemoIgnoresSigterm);
        assert_eq!(request.rate_hz(), 1000);
        assert_eq!(request.lines(), 100_000);
    }

    /// Not a runtime assertion so much as a maintenance trip-wire: adding a
    /// third `HarnessKind` variant makes this `match` non-exhaustive and
    /// fails the build, forcing a deliberate decision here rather than a
    /// silently incomplete catalog.
    #[test]
    fn harness_kind_has_exactly_two_variants() {
        let assert_exhaustive = |kind: HarnessKind| match kind {
            HarnessKind::DemoLines | HarnessKind::DemoIgnoresSigterm => {}
        };
        assert_exhaustive(HarnessKind::DemoLines);
        assert_exhaustive(HarnessKind::DemoIgnoresSigterm);
    }
}

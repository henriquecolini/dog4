//! Sandbox limits and the meter that enforces them while a script runs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::error::{Error, Limit, Result};

/// Resource limits for running scripts. Every field is enforced by the VM.
///
/// [`Limits::default`] is generous and suitable for trusted scripts;
/// [`Limits::sandbox`] is a reasonable starting point for untrusted code.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Instruction budget per top-level run. Loops and calls cost 1; natives
    /// that copy data cost roughly 1 per 64 bytes or elements. `None` = unlimited.
    pub fuel: Option<u64>,
    /// Wall-clock budget per top-level run.
    pub timeout: Option<Duration>,
    /// Maximum number of nested function calls.
    pub max_call_depth: usize,
    /// Maximum nesting of natives calling back into scripts (e.g. `map` inside `map`).
    pub max_native_depth: usize,
    /// Maximum size of any string, in bytes.
    pub max_str_bytes: usize,
    /// Maximum length of any array or map.
    pub max_collection_len: usize,
    /// Maximum bytes written by `print`/`println` per top-level run.
    pub max_output_bytes: Option<usize>,
    /// Maximum syntactic nesting depth of source code, and of displayed/encoded values.
    pub max_nesting: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            fuel: None,
            timeout: None,
            max_call_depth: 100_000,
            max_native_depth: 1_000,
            max_str_bytes: usize::MAX / 2,
            max_collection_len: usize::MAX / 2,
            max_output_bytes: None,
            max_nesting: 512,
        }
    }
}

impl Limits {
    /// Conservative limits for code written by untrusted users.
    pub fn sandbox() -> Limits {
        Limits {
            fuel: Some(5_000_000),
            timeout: Some(Duration::from_secs(2)),
            max_call_depth: 512,
            max_native_depth: 32,
            max_str_bytes: 1 << 20,
            max_collection_len: 100_000,
            max_output_bytes: Some(64 << 10),
            max_nesting: 128,
        }
    }
}

/// A handle the host can use to stop a running script from another thread.
#[derive(Clone, Debug, Default)]
pub struct Interrupt(Arc<AtomicBool>);

impl Interrupt {
    /// Asks the running script to stop as soon as possible.
    pub fn trigger(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

/// How many fuel units pass between the (comparatively slow) clock and interrupt checks.
const CHECK_INTERVAL: u64 = 1 << 12;

/// Tracks resource usage during one top-level run.
#[derive(Debug)]
pub struct Meter {
    fuel: u64,
    until_check: u64,
    deadline: Option<Instant>,
    interrupt: Interrupt,
    /// See [`Limits::max_str_bytes`].
    pub max_str: usize,
    /// See [`Limits::max_collection_len`].
    pub max_len: usize,
    output_left: usize,
}

impl Meter {
    /// A meter for `limits`, starting now.
    pub fn new(limits: &Limits, interrupt: Interrupt) -> Meter {
        Meter {
            fuel: limits.fuel.unwrap_or(u64::MAX),
            until_check: CHECK_INTERVAL,
            deadline: limits.timeout.map(|t| Instant::now() + t),
            interrupt,
            max_str: limits.max_str_bytes,
            max_len: limits.max_collection_len,
            output_left: limits.max_output_bytes.unwrap_or(usize::MAX),
        }
    }

    /// An unlimited meter (used by tests and pure value operations).
    pub fn unlimited() -> Meter {
        Meter::new(&Limits::default(), Interrupt::default())
    }

    /// Spends `n` units of fuel.
    #[inline]
    pub fn tick(&mut self, n: u64) -> Result<()> {
        if self.fuel < n {
            self.fuel = 0;
            return Err(Error::limit(Limit::Fuel));
        }
        self.fuel -= n;
        if self.until_check <= n {
            self.until_check = CHECK_INTERVAL;
            return self.check();
        }
        self.until_check -= n;
        Ok(())
    }

    #[cold]
    fn check(&mut self) -> Result<()> {
        if self.interrupt.take() {
            return Err(Error::limit(Limit::Interrupted));
        }
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(Error::limit(Limit::Timeout));
        }
        Ok(())
    }

    /// Checks that a string of `len` bytes is allowed and pays for building it.
    #[inline]
    pub fn str_len(&mut self, len: usize) -> Result<()> {
        if len > self.max_str {
            return Err(Error::limit(Limit::StrSize));
        }
        self.tick(len as u64 / 64)
    }

    /// Checks that a collection of `len` elements is allowed and pays for building it.
    #[inline]
    pub fn coll_len(&mut self, len: usize) -> Result<()> {
        if len > self.max_len {
            return Err(Error::limit(Limit::CollectionSize));
        }
        self.tick(len as u64 / 64)
    }

    /// Checks that a string may grow to `total` bytes and pays for the `added`
    /// bytes written (appending in place doesn't copy what was already there).
    #[inline]
    pub fn grow_str(&mut self, total: usize, added: usize) -> Result<()> {
        if total > self.max_str {
            return Err(Error::limit(Limit::StrSize));
        }
        self.tick(added as u64 / 64)
    }

    /// Checks that a collection may grow to `total` elements and pays for the
    /// `added` ones.
    #[inline]
    pub fn grow_coll(&mut self, total: usize, added: usize) -> Result<()> {
        if total > self.max_len {
            return Err(Error::limit(Limit::CollectionSize));
        }
        self.tick(added as u64 / 64)
    }

    /// Pays for printing `bytes` bytes.
    pub fn output(&mut self, bytes: usize) -> Result<()> {
        if bytes > self.output_left {
            self.output_left = 0;
            return Err(Error::limit(Limit::Output));
        }
        self.output_left -= bytes;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuel_runs_out() {
        let limits = Limits {
            fuel: Some(10),
            ..Limits::default()
        };
        let mut meter = Meter::new(&limits, Interrupt::default());
        assert!(meter.tick(10).is_ok());
        let err = meter.tick(1).unwrap_err();
        assert_eq!(err.kind(), crate::ErrorKind::Limit(Limit::Fuel));
    }

    #[test]
    fn interrupt_is_noticed() {
        let interrupt = Interrupt::default();
        let mut meter = Meter::new(&Limits::default(), interrupt.clone());
        interrupt.trigger();
        let err = (0..)
            .find_map(|_| meter.tick(1).err())
            .expect("interrupt must fire");
        assert_eq!(err.kind(), crate::ErrorKind::Limit(Limit::Interrupted));
    }

    #[test]
    fn size_caps() {
        let limits = Limits {
            max_str_bytes: 4,
            max_collection_len: 2,
            ..Limits::default()
        };
        let mut meter = Meter::new(&limits, Interrupt::default());
        assert!(meter.str_len(4).is_ok());
        assert!(meter.str_len(5).is_err());
        assert!(meter.coll_len(2).is_ok());
        assert!(meter.coll_len(3).is_err());
    }
}

//! Shared contract harnesses (foundation.md section 5.1): one generic harness per port with
//! more than one implementation, run against each fake and each real adapter alike.
//!
//! A harness is parameterised by a factory that builds the implementation under test from a
//! declarative description. It runs every named case, catches a panic inside a case as that
//! case's failure, and returns a [`ContractReport`] naming each failed case with its expected
//! and actual outcome, so a negative control can assert *which* clause a broken
//! implementation breaks rather than merely that something failed.

pub mod site_content;

use core::fmt;

pub use site_content::{
    LinkTarget, SITE_CONTENT_CASES, SiteContentFactory, SiteEntry, SiteTree,
    check_site_content_contract, site_content_contract,
};

/// One failed case of a contract harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseFailure {
    /// The case's name, one of the harness's case list.
    pub case: &'static str,
    /// Every failed check of the case: the request, what the contract expects and what the
    /// implementation did.
    pub detail: String,
}

/// The outcome of a whole harness run: which cases ran and which failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContractReport {
    cases_run: Vec<&'static str>,
    failures: Vec<CaseFailure>,
}

impl ContractReport {
    /// Records that `case` ran, with `outcome` its result.
    pub(crate) fn record(&mut self, case: &'static str, outcome: Result<(), String>) {
        self.cases_run.push(case);
        if let Err(detail) = outcome {
            self.failures.push(CaseFailure { case, detail });
        }
    }

    /// The names of the cases that ran, in order.
    #[must_use]
    pub fn cases_run(&self) -> &[&'static str] {
        &self.cases_run
    }

    /// The failed cases, in run order.
    #[must_use]
    pub fn failures(&self) -> &[CaseFailure] {
        &self.failures
    }

    /// The names of the failed cases, in run order.
    #[must_use]
    pub fn failed_cases(&self) -> Vec<&'static str> {
        self.failures.iter().map(|failure| failure.case).collect()
    }

    /// Whether every case passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    /// Panics with the full report unless every case passed.
    ///
    /// # Panics
    ///
    /// When a case failed; the message lists every failed case with its details.
    pub fn assert_passed(&self) {
        assert!(self.passed(), "{self}");
    }
}

impl fmt::Display for ContractReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "contract: {} of {} cases failed",
            self.failures.len(),
            self.cases_run.len()
        )?;
        for failure in &self.failures {
            writeln!(f, "case `{}`:\n{}", failure.case, failure.detail)?;
        }
        Ok(())
    }
}

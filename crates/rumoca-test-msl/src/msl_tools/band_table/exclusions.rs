//! The tracked typed exception list a derivation reads, with the digests that
//! bind a table to it (SPEC_0050).

use super::{Path, Result, file_digest};
use crate::msl_tools::common::{
    TRACE_EXCLUSIONS_FILE_REL, load_trace_exclusions_file, typed_exception_reasons,
};
use anyhow::Context;
use std::collections::BTreeMap;
use std::fs;

/// The tracked policy exclusions, keyed by model name, with the digest of the
/// list they came from.
///
/// The list *decides* attribution: an untyped `skipped` entry is a policy
/// exclusion when the model is on this list and a comparator defect when it is
/// not. Reading it must therefore never fall back to "no exclusions" — that
/// default silently reclassifies every policy skip as a defect, and it would do
/// so as a function of the working directory, since the path is resolved from
/// the workspace root found by walking up from the CWD. A list that cannot be
/// read is a hard error, and the digest travels into the table so the reading is
/// attributable after the fact.
#[derive(Debug)]
pub(super) struct TrackedExclusions {
    pub(super) entries: BTreeMap<String, String>,
    pub(super) file: String,
    pub(super) digest: String,
    pub(super) sha256: String,
}

pub(super) fn tracked_exclusions() -> Result<TrackedExclusions> {
    exclusions_from(&crate::repo_root().join(TRACE_EXCLUSIONS_FILE_REL))
}

pub(super) fn exclusions_from(path: &Path) -> Result<TrackedExclusions> {
    let entries = load_trace_exclusions_file(path)
        .map(typed_exception_reasons)
        .with_context(|| {
            format!(
                "cannot attribute policy exclusions without the tracked list '{}'; every `skipped` \
             model would be recorded as a comparator defect instead",
                path.display()
            )
        })?;
    Ok(TrackedExclusions {
        entries,
        file: path.display().to_string(),
        digest: file_digest(path)?,
        sha256: file_sha256(path)?,
    })
}

fn file_sha256(path: &Path) -> Result<String> {
    use sha2::Digest as _;
    let bytes =
        fs::read(path).with_context(|| format!("failed to read '{}' to hash", path.display()))?;
    Ok(format!("{:x}", sha2::Sha256::digest(bytes)))
}

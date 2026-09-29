//! Scan-time contracts: the observation window, what was and was not
//! observed, the budgets a walk runs under, controlled rescan, and capacity
//! watermarks (P1 tasks 2.5, 2.7, 2.9, 2.11, 2.12; specs FS-01 / FS-04 /
//! FS-05 / FS-06 / RT-02 / RT-04).
//!
//! The rule these types exist to enforce: *not observed* is never silently
//! reported as *empty*, and a walk that hit a limit says which limit.

use serde::{Deserialize, Serialize};

/// When a scan ran and under which options. A snapshot is an observation over
/// a window, never a claim about the volume at rest (FS-01).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScanWindow {
    pub started_at_unix_ms: u64,
    pub finished_at_unix_ms: u64,
    /// A stable fingerprint of the options, so two snapshots are only
    /// comparable when the walk was configured identically.
    pub options_fingerprint: String,
    pub scanner_version: String,
}

impl ScanWindow {
    /// How long the walk took, saturating rather than wrapping.
    pub fn duration_ms(&self) -> u64 {
        self.finished_at_unix_ms
            .saturating_sub(self.started_at_unix_ms)
    }
}

/// Why a path was not walked. Each variant is a fact about the walk, not about
/// the file's contents (FS-04).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionReason {
    /// A symbolic link was not followed, so its target stays unobserved.
    LinkNotFollowed,
    /// The entry resolved outside the scan root, so it was refused.
    OutsideRoot,
    /// Another filesystem, which the scan did not cross.
    DifferentVolume,
    /// A hidden entry, excluded by configuration.
    Hidden,
    /// A path component vanished or was unreadable during the walk.
    Unreadable,
    /// The walk hit a budget before reaching it.
    BudgetExhausted,
}

/// One skipped path, recorded rather than dropped.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExcludedPath {
    /// The raw path key, never a display string used as an identity.
    pub locator_key: String,
    pub reason: ExclusionReason,
    /// The walk step at which the decision was made.
    pub step: u64,
}

/// Everything a walk did not manage to observe, kept separately from the
/// nodes it did. "Complete" means this list holds no entry that would change
/// the meaning of the result.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScanExclusions {
    pub paths: Vec<ExcludedPath>,
    /// A short, redacted description of each failure class, without paths.
    pub error_summary: Vec<String>,
}

impl ScanExclusions {
    pub fn push(&mut self, locator_key: String, reason: ExclusionReason, step: u64) {
        self.paths.push(ExcludedPath {
            locator_key,
            reason,
            step,
        });
    }

    pub fn count_of(&self, reason: ExclusionReason) -> usize {
        self.paths
            .iter()
            .filter(|entry| entry.reason == reason)
            .count()
    }

    /// True when nothing was left unobserved in a way that matters.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.error_summary.is_empty()
    }
}

/// What happened to a cloud placeholder during a walk (FS-05). The default is
/// to observe metadata only: a scan must never trigger a download.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceholderPolicy {
    /// Record the placeholder's metadata and leave its bytes alone.
    #[default]
    ObserveMetadataOnly,
    /// Skip the entry entirely and record the exclusion.
    Skip,
    /// Hydrate it. Refused by the walk: downloads are not a scan's business.
    Hydrate,
}

impl PlaceholderPolicy {
    /// Hydration is never available to a walk, whatever a caller asks for.
    pub fn resolve(requested: PlaceholderPolicy) -> Self {
        match requested {
            PlaceholderPolicy::Hydrate => PlaceholderPolicy::ObserveMetadataOnly,
            other => other,
        }
    }

    pub fn may_hydrate(self) -> bool {
        matches!(self, PlaceholderPolicy::Hydrate)
    }
}

/// The limits a walk runs under (RT-02). Every field is a ceiling: a walk that
/// reaches one stops and records which, rather than quietly returning less.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScanBudget {
    pub max_nodes: u64,
    pub max_duration_ms: u64,
    pub max_staging_bytes: u64,
    /// Nodes written per batch, so a slow consumer cannot force one huge
    /// transaction.
    pub write_batch_nodes: u64,
}

impl Default for ScanBudget {
    fn default() -> Self {
        Self {
            max_nodes: 5_000_000,
            max_duration_ms: 600_000,
            max_staging_bytes: 2 << 30,
            write_batch_nodes: 512,
        }
    }
}

impl ScanBudget {
    pub fn validated(self) -> Result<Self, BudgetFault> {
        if self.max_nodes == 0
            || self.max_duration_ms == 0
            || self.max_staging_bytes == 0
            || self.write_batch_nodes == 0
        {
            return Err(BudgetFault::Invalid);
        }
        Ok(self)
    }
}

/// Why a walk stopped early, if it did.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetFault {
    /// A budget was configured with a zero ceiling.
    Invalid,
}

/// Live counters a walk keeps while it runs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BudgetUsage {
    pub nodes: u64,
    pub staged_bytes: u64,
    pub elapsed_ms: u64,
}

/// The verdict after each step: keep walking, or stop for a named reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetDecision {
    Continue,
    Stop(ScanBudgetStop),
}

impl BudgetDecision {
    pub fn is_stop(self) -> bool {
        matches!(self, Self::Stop(_))
    }
}

/// The named limit that ended a walk.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanBudgetStop {
    NodeLimit,
    TimeLimit,
    StagingLimit,
    Cancelled,
}

impl ScanBudget {
    /// Accounts one more node and decides whether the walk may continue.
    pub fn charge_node(&self, usage: &mut BudgetUsage, node_bytes: u64) -> BudgetDecision {
        usage.nodes = usage.nodes.saturating_add(1);
        if usage.nodes > self.max_nodes {
            return BudgetDecision::Stop(ScanBudgetStop::NodeLimit);
        }
        usage.staged_bytes = usage.staged_bytes.saturating_add(node_bytes);
        if usage.staged_bytes > self.max_staging_bytes {
            return BudgetDecision::Stop(ScanBudgetStop::StagingLimit);
        }
        if usage.elapsed_ms > self.max_duration_ms {
            return BudgetDecision::Stop(ScanBudgetStop::TimeLimit);
        }
        BudgetDecision::Continue
    }

    /// A cancellation observed between batches.
    pub fn observe_cancel(cancelled: bool) -> Option<BudgetDecision> {
        cancelled.then_some(BudgetDecision::Stop(ScanBudgetStop::Cancelled))
    }
}

/// The comparison a rescan performs against the previous snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RescanComparison {
    pub previous_nodes: u64,
    pub fresh_nodes: u64,
    pub refreshed: u64,
    /// Paths the previous snapshot saw that the rescan did not.
    pub missing: u64,
    /// Paths that appeared since the previous snapshot.
    pub added: u64,
    /// True when nothing was missing, so no deletion may be inferred.
    pub complete_observation: bool,
}

/// Compares a fresh observation against a previous one. A path missing from
/// the new walk is counted, never deleted, unless the walk itself was cut
/// short.
pub fn compare_rescan(
    previous_count: u64,
    fresh_count: u64,
    previous_paths: &[String],
    fresh_paths: &[String],
    walk_was_complete: bool,
) -> RescanComparison {
    let fresh: std::collections::HashSet<&str> = fresh_paths.iter().map(String::as_str).collect();
    let previous: std::collections::HashSet<&str> =
        previous_paths.iter().map(String::as_str).collect();
    let kept = previous.iter().filter(|path| fresh.contains(*path)).count();
    let missing = previous.len() - kept;
    let added = fresh.len() - kept;
    let refreshed = kept;
    RescanComparison {
        previous_nodes: previous_count,
        fresh_nodes: fresh_count,
        refreshed: refreshed as u64,
        missing: missing as u64,
        added: added as u64,
        // Only a complete walk may treat a missing path as a real absence.
        complete_observation: walk_was_complete,
    }
}

/// Where the engine's bytes live, so capacity can be measured per area rather
/// than as one number (RT-04).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageArea {
    GraphDatabase,
    ControlDatabase,
    WriteAheadLog,
    Staging,
    Backups,
    Logs,
    Quarantine,
}

/// A watermark threshold at which new work is refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Watermark {
    /// The ceiling at which new work is refused, in bytes.
    pub refuse_above_bytes: u64,
    /// The level at which a warning is recorded.
    pub warn_above_bytes: u64,
}

impl Watermark {
    pub fn is_valid(self) -> bool {
        self.warn_above_bytes <= self.refuse_above_bytes
    }

    /// The area this watermark guards.
    pub fn verdict(self, used: u64) -> WatermarkVerdict {
        if used > self.refuse_above_bytes {
            WatermarkVerdict::Refuse
        } else if used > self.warn_above_bytes {
            WatermarkVerdict::Warn
        } else {
            WatermarkVerdict::Ok
        }
    }
}

/// What a watermark says about accepting new work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatermarkVerdict {
    Ok,
    Warn,
    /// New work is refused; existing data and recovery records are untouched.
    Refuse,
}

impl WatermarkVerdict {
    pub fn accepts_new_work(self) -> bool {
        !matches!(self, Self::Refuse)
    }
}

/// A per-area capacity reading, so a report can say which area filled up
/// instead of one opaque total.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapacityReading {
    pub area: StorageArea,
    pub used_bytes: u64,
    pub watermark: Watermark,
    pub verdict: WatermarkVerdict,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_reports_its_own_duration_without_wrapping() {
        let window = ScanWindow {
            started_at_unix_ms: 1_000,
            finished_at_unix_ms: 1_500,
            options_fingerprint: "fp".into(),
            scanner_version: "test".into(),
        };
        assert_eq!(window.duration_ms(), 500);
        let backwards = ScanWindow {
            finished_at_unix_ms: 10,
            started_at_unix_ms: 100,
            ..window
        };
        assert_eq!(backwards.duration_ms(), 0, "a backwards clock saturates");
    }

    #[test]
    fn exclusions_are_countable_by_reason() {
        let mut exclusions = ScanExclusions::default();
        exclusions.push("a".into(), ExclusionReason::LinkNotFollowed, 1);
        exclusions.push("b".into(), ExclusionReason::LinkNotFollowed, 2);
        exclusions.push("c".into(), ExclusionReason::OutsideRoot, 3);
        assert_eq!(exclusions.count_of(ExclusionReason::LinkNotFollowed), 2);
        assert_eq!(exclusions.count_of(ExclusionReason::OutsideRoot), 1);
        assert!(!exclusions.is_empty());
        assert!(ScanExclusions::default().is_empty());
    }

    #[test]
    fn a_placeholder_is_never_hydrated_by_a_walk() {
        // Even a caller that asks for hydration is downgraded: downloading a
        // file is not a scan's job.
        assert_eq!(
            PlaceholderPolicy::resolve(PlaceholderPolicy::Hydrate),
            PlaceholderPolicy::ObserveMetadataOnly
        );
        assert!(!PlaceholderPolicy::resolve(PlaceholderPolicy::Hydrate).may_hydrate());
        assert!(!PlaceholderPolicy::default().may_hydrate());
    }

    #[test]
    fn budgets_stop_a_walk_for_a_named_reason() {
        let budget = ScanBudget {
            max_nodes: 2,
            max_duration_ms: 1_000,
            max_staging_bytes: 1_000_000,
            write_batch_nodes: 1,
        };
        let mut usage = BudgetUsage::default();
        assert_eq!(budget.charge_node(&mut usage, 10), BudgetDecision::Continue);
        assert_eq!(budget.charge_node(&mut usage, 10), BudgetDecision::Continue);
        assert_eq!(
            budget.charge_node(&mut usage, 10),
            BudgetDecision::Stop(ScanBudgetStop::NodeLimit)
        );
        assert_eq!(usage.nodes, 3);
    }

    #[test]
    fn staging_and_time_limits_are_reported_separately() {
        let node_limited = ScanBudget {
            max_nodes: 100,
            max_duration_ms: 1_000,
            max_staging_bytes: 1_000_000,
            write_batch_nodes: 1,
        };
        let mut usage = BudgetUsage {
            nodes: 1,
            staged_bytes: 999_999,
            elapsed_ms: 10,
        };
        assert_eq!(
            node_limited.charge_node(&mut usage, 10_000),
            BudgetDecision::Stop(ScanBudgetStop::StagingLimit)
        );

        let time_limited = ScanBudget {
            max_nodes: 100,
            max_duration_ms: 5,
            max_staging_bytes: 1_000_000,
            write_batch_nodes: 1,
        };
        let mut usage = BudgetUsage {
            nodes: 1,
            staged_bytes: 0,
            elapsed_ms: 10,
        };
        assert_eq!(
            time_limited.charge_node(&mut usage, 1),
            BudgetDecision::Stop(ScanBudgetStop::TimeLimit)
        );
    }

    #[test]
    fn a_zero_budget_is_refused_rather_than_walking_forever() {
        let budget = ScanBudget {
            max_nodes: 0,
            ..ScanBudget::default()
        };
        assert_eq!(budget.validated().unwrap_err(), BudgetFault::Invalid);
        assert!(ScanBudget::default().validated().is_ok());
    }

    #[test]
    fn cancellation_is_a_named_stop_rather_than_a_silent_finish() {
        assert_eq!(
            ScanBudget::observe_cancel(true),
            Some(BudgetDecision::Stop(ScanBudgetStop::Cancelled))
        );
        assert_eq!(ScanBudget::observe_cancel(false), None);
    }

    #[test]
    fn a_rescan_counts_a_missing_path_without_deleting_it() {
        let previous = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let fresh = vec!["a".to_string(), "b".to_string(), "d".to_string()];
        let comparison = compare_rescan(3, 3, &previous, &fresh, true);
        assert_eq!(comparison.missing, 1, "c is missing from the fresh walk");
        assert_eq!(comparison.added, 1, "d appeared");
        assert_eq!(comparison.refreshed, 2);
        assert!(comparison.complete_observation);

        // A walk that was cut short cannot assert absence.
        let partial = compare_rescan(3, 2, &previous, &fresh, false);
        assert!(!partial.complete_observation);
    }

    #[test]
    fn watermarks_refuse_new_work_only_past_the_hard_ceiling() {
        let watermark = Watermark {
            warn_above_bytes: 100,
            refuse_above_bytes: 200,
        };
        assert!(watermark.is_valid());
        assert_eq!(watermark.verdict(50), WatermarkVerdict::Ok);
        assert_eq!(watermark.verdict(150), WatermarkVerdict::Warn);
        assert_eq!(watermark.verdict(250), WatermarkVerdict::Refuse);
        assert!(watermark.verdict(50).accepts_new_work());
        assert!(!watermark.verdict(250).accepts_new_work());

        // An inverted watermark is not usable.
        assert!(
            !Watermark {
                warn_above_bytes: 300,
                refuse_above_bytes: 200
            }
            .is_valid()
        );
    }
}

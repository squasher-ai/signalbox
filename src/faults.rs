//! Releases and fault windows.
//!
//! Every decision here depends only on the seed, the scenario index, and the
//! configuration, so faults keep shard bytes reproducible. Offsets are
//! seconds from `start_ns`. A scenario is inside a window when its start time
//! is in `[start_s, end_s)`.

use crate::{
    config::{Config, Preset, valid_label},
    error::Error,
    model::random,
};
use clap::ValueEnum;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const NS_PER_SECOND: u128 = 1_000_000_000;
/// Extra WARN logs per scenario inside a `log_burst` window.
pub const BURST_LOGS_PER_RECORD: u64 = 20;
const MAX_ITEMS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Release {
    /// Seconds from `start_ns` at which this version starts serving.
    pub at_s: u64,
    #[schemars(length(min = 1, max = 256))]
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    /// The work span (model call or database query) becomes six times slower.
    LatencyRegression,
    /// Requests fail with an unhandled exception.
    ErrorSpike,
    /// Database queries slow down by 1.5-3 s; one in four times out.
    DbSaturation,
    /// The upstream (model provider, or the API behind the gateway) returns 503.
    UpstreamOutage,
    /// Each scenario emits 20 extra correlated WARN logs. Does not use `rate`.
    LogBurst,
    /// The model provider returns 429 rate limit errors.
    GenaiRateLimit,
    /// The model request exceeds the context window and fails with 400.
    GenaiContextLength,
    /// The model response is filtered (`finish_reason` = `content_filter`).
    GenaiRefusal,
    /// The tool call fails; the agent invocation completes.
    GenaiToolError,
}

impl FaultKind {
    pub fn default_rate(self) -> u8 {
        match self {
            Self::LatencyRegression | Self::DbSaturation | Self::LogBurst => 100,
            Self::UpstreamOutage => 90,
            Self::GenaiRateLimit => 60,
            Self::GenaiToolError => 50,
            Self::ErrorSpike => 40,
            Self::GenaiContextLength => 30,
            Self::GenaiRefusal => 25,
        }
    }

    pub fn supports(self, preset: Preset) -> bool {
        match self {
            Self::LatencyRegression | Self::ErrorSpike | Self::LogBurst => true,
            Self::DbSaturation => !preset.is_genai(),
            Self::UpstreamOutage => preset != Preset::Web,
            Self::GenaiRateLimit
            | Self::GenaiContextLength
            | Self::GenaiRefusal
            | Self::GenaiToolError => preset.is_genai(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fault {
    pub kind: FaultKind,
    /// Window start in seconds from `start_ns` (inclusive).
    pub start_s: u64,
    /// Window end in seconds from `start_ns` (exclusive).
    pub end_s: u64,
    /// Percent of scenarios in the window that are affected; default per kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 100))]
    pub rate: Option<u8>,
    /// Only affect scenarios served by this release version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 256))]
    pub version: Option<String>,
}

impl Fault {
    pub fn rate(&self) -> u8 {
        self.rate.unwrap_or(self.kind.default_rate())
    }
}

/// Named fault scenarios for `--fault`. Windows scale with the run length;
/// see [`apply_presets`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FaultPreset {
    /// Release a new version at 50% of the run (or at the offset of a
    /// configured release of that version); its work span is 6x slower.
    ReleaseRegression,
    ErrorSpike,
    DbSaturation,
    UpstreamOutage,
    LogBurst,
    GenaiRateLimit,
    GenaiContextLength,
    GenaiRefusal,
    GenaiToolError,
}

impl FaultPreset {
    pub fn kind(self) -> FaultKind {
        match self {
            Self::ReleaseRegression => FaultKind::LatencyRegression,
            Self::ErrorSpike => FaultKind::ErrorSpike,
            Self::DbSaturation => FaultKind::DbSaturation,
            Self::UpstreamOutage => FaultKind::UpstreamOutage,
            Self::LogBurst => FaultKind::LogBurst,
            Self::GenaiRateLimit => FaultKind::GenaiRateLimit,
            Self::GenaiContextLength => FaultKind::GenaiContextLength,
            Self::GenaiRefusal => FaultKind::GenaiRefusal,
            Self::GenaiToolError => FaultKind::GenaiToolError,
        }
    }
}

/// Replace `config.faults` with the named presets.
///
/// Other presets share the span from 40% to 90% of the run in equal,
/// consecutive windows, in the order given, so each has a clean baseline
/// before and a recovery after. `release-regression` adds a release at 50%
/// that stays slow to the end (or uses the configured release of the same
/// version, at its own offset); with it, the other presets share 10% to 45%
/// instead, so no window overlaps a release at 50%.
pub fn apply_presets(config: &mut Config, presets: &[FaultPreset]) {
    let total = (config.duration_ns() as u128 / NS_PER_SECOND) as u64;
    let others: Vec<_> =
        presets.iter().filter(|preset| **preset != FaultPreset::ReleaseRegression).collect();
    let release = presets.contains(&FaultPreset::ReleaseRegression);
    // Keep other faults clear of the release so both stay attributable.
    let (first, span) =
        if release { (total / 10, total * 7 / 20) } else { (total * 2 / 5, total / 2) };
    let mut faults: Vec<Fault> = others
        .iter()
        .enumerate()
        .map(|(slot, preset)| {
            let share = |slot: usize| first + span * slot as u64 / others.len() as u64;
            let start_s = share(slot);
            Fault {
                kind: preset.kind(),
                start_s,
                end_s: share(slot + 1).max(start_s + 1),
                rate: None,
                version: None,
            }
        })
        .collect();
    if release {
        let version = next_version(&config.service_version);
        // A configured release of the same version keeps its own offset, and
        // the regression starts there, so the label matches the release.
        let at_s = match config.releases.iter().find(|release| release.version == version) {
            Some(existing) => existing.at_s,
            None => {
                config.releases.push(Release { at_s: total / 2, version: version.clone() });
                config.releases.sort_by_key(|release| release.at_s);
                total / 2
            }
        };
        faults.push(Fault {
            kind: FaultKind::LatencyRegression,
            start_s: at_s,
            end_s: total + 1,
            rate: None,
            version: Some(version),
        });
    }
    config.faults = faults;
}

/// `1.4.2` becomes `1.5.0`; other version strings get a `-next` suffix.
fn next_version(version: &str) -> String {
    let parts: Vec<_> = version.split('.').collect();
    match parts.as_slice() {
        [major, minor, patch]
            if [major, minor, patch].iter().all(|part| part.parse::<u64>().is_ok()) =>
        {
            format!("{major}.{}.0", minor.parse::<u64>().unwrap_or(0).saturating_add(1))
        }
        _ => format!("{version}-next"),
    }
}

pub fn validate(config: &Config) -> Result<(), Error> {
    let invalid = |message: String| Err(Error::new("invalid_config", message));
    if config.releases.len() > MAX_ITEMS || config.faults.len() > MAX_ITEMS {
        return invalid(format!("releases and faults are limited to {MAX_ITEMS} entries each"));
    }
    let run_end = config.duration_ns() as u128;
    let mut versions = vec![config.service_version.as_str()];
    for (position, release) in config.releases.iter().enumerate() {
        if !valid_label(&release.version) || versions.contains(&release.version.as_str()) {
            return invalid(format!(
                "releases[{position}].version must be a new label of 1..=256 characters"
            ));
        }
        if position > 0 && release.at_s <= config.releases[position - 1].at_s {
            return invalid("releases must be in strictly increasing at_s order".into());
        }
        if release.at_s as u128 * NS_PER_SECOND >= run_end {
            return invalid(format!("releases[{position}].at_s must be before the end of the run"));
        }
        versions.push(&release.version);
    }
    for (position, fault) in config.faults.iter().enumerate() {
        let name = format!("faults[{position}]");
        if fault.start_s >= fault.end_s || fault.start_s as u128 * NS_PER_SECOND >= run_end {
            return invalid(format!(
                "{name} needs start_s < end_s and must start before the end of the run"
            ));
        }
        if !matches!(fault.rate, None | Some(1..=100))
            || fault.kind == FaultKind::LogBurst && !matches!(fault.rate, None | Some(100))
        {
            return invalid(format!("{name}.rate must be 1..=100 (log_burst accepts only 100)"));
        }
        if fault.version.as_deref().is_some_and(|version| !versions.contains(&version)) {
            return invalid(format!("{name}.version must be service_version or a release"));
        }
        if !fault.kind.supports(config.preset) {
            return invalid(format!("{name} kind is not supported by the selected preset"));
        }
    }
    Ok(())
}

/// First scenario index at or after `offset_s`, clamped to `count`.
pub fn index_at(config: &Config, offset_s: u64) -> u64 {
    (offset_s as u128 * NS_PER_SECOND)
        .div_ceil(config.interval_ns as u128)
        .min(config.count as u128) as u64
}

/// Version slot: 0 for `service_version`, `n` for `releases[n - 1]`.
pub fn version_slot(config: &Config, index: u64) -> usize {
    config.releases.iter().take_while(|release| index_at(config, release.at_s) <= index).count()
}

pub fn version_name(config: &Config, slot: usize) -> &str {
    match slot {
        0 => &config.service_version,
        slot => &config.releases[slot - 1].version,
    }
}

/// Scenario indices `[first, end)` served by a version slot.
pub fn version_range(config: &Config, slot: usize) -> (u64, u64) {
    let first = if slot == 0 { 0 } else { index_at(config, config.releases[slot - 1].at_s) };
    let end = config.releases.get(slot).map_or(config.count, |next| index_at(config, next.at_s));
    (first, end.max(first))
}

/// Scenario indices `[first, end)` inside a fault window.
pub fn window(config: &Config, fault: &Fault) -> (u64, u64) {
    (index_at(config, fault.start_s), index_at(config, fault.end_s))
}

fn applies(config: &Config, fault: &Fault, index: u64, version: usize) -> bool {
    let (first, end) = window(config, fault);
    (first..end).contains(&index)
        && fault.version.as_deref().is_none_or(|wanted| version_name(config, version) == wanted)
}

/// The exclusive fault for a scenario: the first listed fault whose window,
/// version, and rate draw match. `log_burst` is additive and never selected.
pub fn select(config: &Config, index: u64, version: usize) -> Option<usize> {
    config.faults.iter().enumerate().position(|(position, fault)| {
        fault.kind != FaultKind::LogBurst
            && applies(config, fault, index, version)
            && random(config.seed, index, 200 + position as u64) % 100 < fault.rate() as u64
    })
}

/// Extra logs for a scenario from all matching `log_burst` faults.
pub fn burst_logs(config: &Config, index: u64) -> u64 {
    if config.faults.is_empty() {
        return 0;
    }
    let version = version_slot(config, index);
    config
        .faults
        .iter()
        .filter(|fault| fault.kind == FaultKind::LogBurst && applies(config, fault, index, version))
        .count() as u64
        * BURST_LOGS_PER_RECORD
}

/// Total extra logs for the run, computed from index ranges without scanning.
pub fn total_burst_logs(config: &Config) -> u64 {
    config
        .faults
        .iter()
        .filter(|fault| fault.kind == FaultKind::LogBurst)
        .map(|fault| {
            let (first, end) = window(config, fault);
            (0..=config.releases.len())
                .filter(|slot| {
                    fault.version.as_deref().is_none_or(|v| version_name(config, *slot) == v)
                })
                .map(|slot| {
                    let (from, to) = version_range(config, slot);
                    to.min(end).saturating_sub(from.max(first))
                })
                .sum::<u64>()
        })
        .sum::<u64>()
        * BURST_LOGS_PER_RECORD
}

/// Count affected scenarios per fault for the indices `[start, end)`.
pub fn tally(config: &Config, start: u64, end: u64) -> Vec<u64> {
    let mut hits = vec![0; config.faults.len()];
    if hits.is_empty() {
        return hits;
    }
    for index in start..end {
        let version = version_slot(config, index);
        if let Some(fault) = select(config, index, version) {
            hits[fault] += 1;
        }
        for (position, fault) in config.faults.iter().enumerate() {
            if fault.kind == FaultKind::LogBurst && applies(config, fault, index, version) {
                hits[position] += 1;
            }
        }
    }
    hits
}

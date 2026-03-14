use crate::migration::types::{MigrationResult, MigrationStatus};

pub fn compute_overall_score(results: &[MigrationResult]) -> f64 {
    if results.is_empty() {
        return 0.0;
    }
    let total: f64 = results.iter().map(|r| r.score as f64).sum();
    total / results.len() as f64
}

pub struct StatusBreakdown {
    pub native: usize,
    pub compatible: usize,
    pub partial: usize,
    pub unsupported: usize,
    pub unknown: usize,
}

impl StatusBreakdown {
    pub fn from_results(results: &[MigrationResult]) -> Self {
        let mut b = StatusBreakdown {
            native: 0,
            compatible: 0,
            partial: 0,
            unsupported: 0,
            unknown: 0,
        };
        for r in results {
            match r.status {
                MigrationStatus::Native => b.native += 1,
                MigrationStatus::Compatible => b.compatible += 1,
                MigrationStatus::Partial => b.partial += 1,
                MigrationStatus::Unsupported => b.unsupported += 1,
                MigrationStatus::Unknown => b.unknown += 1,
            }
        }
        b
    }

}

pub fn top_blockers(results: &[MigrationResult]) -> Vec<(String, usize)> {
    use std::collections::HashMap;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for r in results {
        if r.status == MigrationStatus::Unsupported {
            *counts.entry(r.resource_type.clone()).or_insert(0) += 1;
        }
    }
    let mut sorted: Vec<(String, usize)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    sorted.truncate(3);
    sorted
}

pub fn migration_recommendation(score: f64) -> &'static str {
    match score as u8 {
        90..=100 => "Excellent! Nearly full automation possible. Ready to migrate.",
        75..=89 => "Good coverage. Minor manual adjustments needed after generation.",
        60..=74 => "Moderate coverage. Review COMPATIBLE resources and update TODOs.",
        40..=59 => "Partial migration possible. Significant manual work for unsupported resources.",
        _ => "Low migration score. Many resources require manual replacement or architectural changes.",
    }
}

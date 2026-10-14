//! Audience-coverage reader rule. Membership is resolved at read time.

use std::collections::BTreeSet;

/// Scope metadata from a trusted canonical graph, not from record prose.
#[derive(Clone, Debug)]
pub struct ReaderRecord {
    /// Authenticated profile label.
    pub profile: String,
    /// Native-stamped place label.
    pub place: String,
    /// Set only for the exact owner-approved standing core version.
    pub approved_core_agent: Option<String>,
    /// Retracted records cannot reach any reader.
    pub retracted: bool,
    /// Every source-record scope must also pass.
    pub derived_from: Vec<String>,
}

/// Current trusted reader and source graph.
pub trait ReaderWorld {
    /// Resolve current readers for a native-stamped place, failing closed if unknown.
    fn readers(&self, place: &str) -> Option<BTreeSet<String>>;
    /// Resolve exact record metadata; absent/deleted sources fail closed.
    fn record(&self, id: &str) -> Option<ReaderRecord>;
}

/// The authenticated summon whose entire audience will receive the answer.
#[derive(Clone, Debug)]
pub struct Summon {
    /// Bound profile.
    pub profile: String,
    /// Authenticated agent.
    pub agent: String,
    /// Current verified readers of the destination conversation.
    pub audience: BTreeSet<String>,
}

/// Check profile, current audience, core approval and every source before ranking.
pub fn may_read(world: &impl ReaderWorld, record_id: &str, summon: &Summon) -> bool {
    if summon.audience.is_empty() || !summon.audience.contains(&summon.agent) {
        return false;
    }
    let mut visiting = BTreeSet::new();
    let mut remaining = 1024;
    visit(world, record_id, summon, &mut visiting, &mut remaining, 0)
}

fn visit(
    world: &impl ReaderWorld,
    id: &str,
    summon: &Summon,
    visiting: &mut BTreeSet<String>,
    remaining: &mut usize,
    depth: usize,
) -> bool {
    if *remaining == 0 || depth > 64 || !visiting.insert(id.to_owned()) {
        return false;
    }
    *remaining -= 1;
    let allowed = if let Some(record) = world.record(id) {
        record.profile == summon.profile
            && !record.retracted
            && record.derived_from.len() <= 32
            && match &record.approved_core_agent {
                Some(agent) => agent == &summon.agent,
                None => world
                    .readers(&record.place)
                    .is_some_and(|readers| readers.is_superset(&summon.audience)),
            }
            && record
                .derived_from
                .iter()
                .all(|source| visit(world, source, summon, visiting, remaining, depth + 1))
    } else {
        false
    };
    visiting.remove(id);
    allowed
}

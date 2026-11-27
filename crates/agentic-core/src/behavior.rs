//! Read-only behavioral analysis of event traces. Exposure and declared reuse are
//! not causal influence; validations remain scoped to their named checker.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Version of newly recorded behavioral traces.
pub const SCHEMA_VERSION: u32 = 2;
/// Version of the deterministic analyzer, independent of the input schema.
pub const ANALYZER_VERSION: &str = "1.0.0";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub timestamp: String,
    pub agent_id: Option<String>,
    pub kind: String,
    pub message: String,
    pub data: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub agent_id: String,
    /// Hypothesis/claim, not an independently established fact.
    pub text: String,
    /// Legacy single-parent field retained for v1 histories.
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub parent_ids: Vec<String>,
    /// References to observations in the same experiment's event stream.
    #[serde(default)]
    pub evidence_seqs: Vec<u64>,
    /// Optional answer consumed by a task-specific trusted checker.
    #[serde(default)]
    pub answer: Option<i64>,
    pub timestamp: String,
}
impl Finding {
    pub fn parents(&self) -> BTreeSet<String> {
        self.parent_ids
            .iter()
            .chain(self.parent.iter())
            .cloned()
            .collect()
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub findings: usize,
    pub readers: usize,
    pub cross_agent_exposures: usize,
    pub declared_reuses: usize,
    pub combinations: usize,
    pub verified_findings: usize,
    pub rejected_findings: usize,
    pub declared_strategy_changes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Link {
    pub from: String,
    pub to: String,
    /// published, exposed, declared_reuse, evidence, validated or rejected.
    pub relation: String,
    pub event_seq: Option<u64>,
    /// None for histories which cannot substantiate availability at publication.
    pub exposure_seq: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step {
    pub event_seq: u64,
    pub timestamp: String,
    pub kind: String,
    pub message: String,
    /// runtime_observation or agent_declaration; never hidden model reasoning.
    pub source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub analyzer_version: String,
    pub through_seq: u64,
    pub metrics: Metrics,
    pub links: Vec<Link>,
    pub trajectories: BTreeMap<String, Vec<Step>>,
    pub warnings: Vec<String>,
}
/// Analyze immutable events. Does not invoke a model or alter the agents' world.
pub fn analyze(events: &[Event], findings: &[Finding]) -> Report {
    let mut report = Report {
        analyzer_version: ANALYZER_VERSION.into(),
        through_seq: events.last().map_or(0, |e| e.seq),
        metrics: Metrics {
            findings: findings.len(),
            ..Metrics::default()
        },
        links: vec![],
        trajectories: BTreeMap::new(),
        warnings: vec![],
    };
    let by_id: BTreeMap<_, _> = findings.iter().map(|f| (f.id.as_str(), f)).collect();
    let mut published = BTreeMap::new();
    let mut exposures: BTreeMap<(String, String), u64> = BTreeMap::new();
    let mut readers = BTreeSet::new();
    let mut verified = BTreeSet::new();
    let mut rejected = BTreeSet::new();
    for e in events {
        if let Some(agent) = &e.agent_id {
            report
                .trajectories
                .entry(agent.clone())
                .or_default()
                .push(Step {
                    event_seq: e.seq,
                    timestamp: e.timestamp.clone(),
                    kind: e.kind.clone(),
                    message: e.message.clone(),
                    source: if e.kind.starts_with("strategy.")
                        || e.kind == "finding.created"
                        || e.kind == "memory.published"
                    {
                        "agent_declaration"
                    } else {
                        "runtime_observation"
                    }
                    .into(),
                });
            if e.kind == "memory.read" {
                for id in e.data["finding_ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    if let Some(f) = by_id.get(id) {
                        if f.agent_id != *agent
                            && !exposures.contains_key(&(id.into(), agent.clone()))
                        {
                            exposures.insert((id.into(), agent.clone()), e.seq);
                            readers.insert(agent.clone());
                            report.links.push(Link {
                                from: format!("finding:{id}"),
                                to: format!("agent:{agent}"),
                                relation: "exposed".into(),
                                event_seq: Some(e.seq),
                                exposure_seq: Some(e.seq),
                            });
                        }
                    }
                }
            }
            if e.kind == "strategy.changed" {
                report.metrics.declared_strategy_changes += 1;
            }
        }
        if e.kind == "finding.created" || e.kind == "memory.published" {
            if let Some(id) = e.data["id"].as_str() {
                published.insert(id.to_owned(), e.seq);
            }
        }
        if e.kind == "finding.validated" && e.data["source"] == "trusted_checker" {
            if let Some(id) = e.data["finding_id"]
                .as_str()
                .filter(|id| by_id.contains_key(id))
            {
                let valid = e.data["valid"].as_bool();
                if let Some(valid) = valid {
                    if valid {
                        verified.insert(id.to_owned());
                    } else {
                        rejected.insert(id.to_owned());
                    }
                    report.links.push(Link {
                        from: format!("finding:{id}"),
                        to: format!("event:{}", e.seq),
                        relation: if valid { "validated" } else { "rejected" }.into(),
                        event_seq: Some(e.seq),
                        exposure_seq: None,
                    });
                }
            }
        }
    }
    for f in findings {
        let seq = published.get(&f.id).copied();
        report.links.push(Link {
            from: format!("agent:{}", f.agent_id),
            to: format!("finding:{}", f.id),
            relation: "published".into(),
            event_seq: seq,
            exposure_seq: None,
        });
        let parents = f.parents();
        if parents.len() > 1 {
            report.metrics.combinations += 1;
        }
        for id in parents {
            if let Some(parent) = by_id.get(id.as_str()) {
                let exposure = exposures
                    .get(&(id.clone(), f.agent_id.clone()))
                    .copied()
                    .filter(|r| seq.is_some_and(|s| *r < s));
                if parent.agent_id != f.agent_id {
                    report.metrics.declared_reuses += 1;
                    if exposure.is_none() {
                        report.warnings.push(format!(
                            "Reprise {} → {} sans lecture préalable documentée.",
                            id, f.id
                        ));
                    }
                }
                report.links.push(Link {
                    from: format!("finding:{id}"),
                    to: format!("finding:{}", f.id),
                    relation: "declared_reuse".into(),
                    event_seq: seq,
                    exposure_seq: exposure,
                });
            } else {
                report.warnings.push(format!("Parent absent : {id}"));
            }
        }
        for evidence in &f.evidence_seqs {
            report.links.push(Link {
                from: format!("event:{evidence}"),
                to: format!("finding:{}", f.id),
                relation: "evidence".into(),
                event_seq: Some(*evidence),
                exposure_seq: None,
            });
        }
    }
    report.metrics.readers = readers.len();
    report.metrics.cross_agent_exposures = exposures.len();
    report.metrics.verified_findings = verified.len();
    report.metrics.rejected_findings = rejected.len();
    report
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn event(seq: u64, agent: &str, kind: &str, data: Value) -> Event {
        Event {
            seq,
            agent_id: Some(agent.into()),
            kind: kind.into(),
            data,
            timestamp: String::new(),
            message: String::new(),
        }
    }
    fn finding(id: &str, agent: &str, parents: Vec<&str>) -> Finding {
        Finding {
            id: id.into(),
            agent_id: agent.into(),
            text: "claim".into(),
            parent: None,
            parent_ids: parents.into_iter().map(str::to_owned).collect(),
            evidence_seqs: vec![],
            answer: None,
            timestamp: String::new(),
        }
    }
    #[test]
    fn reading_is_not_reuse_and_late_reading_is_not_prior_exposure() {
        let f = vec![finding("x", "a", vec![]), finding("y", "b", vec!["x"])];
        let events = vec![
            event(1, "a", "finding.created", json!({"id":"x"})),
            event(2, "b", "finding.created", json!({"id":"y"})),
            event(3, "b", "memory.read", json!({"finding_ids":["x"]})),
            event(4, "b", "memory.read", json!({"finding_ids":["x"]})),
        ];
        let r = analyze(&events, &f);
        assert_eq!(r.metrics.cross_agent_exposures, 1);
        assert_eq!(r.metrics.declared_reuses, 1);
        assert_eq!(r.warnings.len(), 1);
        assert!(r
            .links
            .iter()
            .find(|l| l.relation == "declared_reuse")
            .unwrap()
            .exposure_seq
            .is_none());
        let r = analyze(&events, &f[..1]);
        assert_eq!(r.metrics.declared_reuses, 0);
    }
    #[test]
    fn verifies_combinations_only_with_trusted_checker_events() {
        let f = vec![
            finding("x", "a", vec![]),
            finding("y", "b", vec![]),
            finding("z", "c", vec!["x", "y"]),
        ];
        let events = vec![
            event(1, "c", "memory.read", json!({"finding_ids":["x","y"]})),
            event(2, "c", "finding.created", json!({"id":"z"})),
            event(
                3,
                "c",
                "finding.confirmed",
                json!({"finding_id":"z","valid":true}),
            ),
            event(
                4,
                "c",
                "finding.validated",
                json!({"finding_id":"z","valid":true,"source":"trusted_checker"}),
            ),
        ];
        let r = analyze(&events, &f);
        assert_eq!(r.metrics.combinations, 1);
        assert_eq!(r.metrics.verified_findings, 1);
        assert_eq!(r.metrics.declared_reuses, 2);
        assert!(r.warnings.is_empty());
        assert_eq!(analyze(&events[..3], &f).metrics.verified_findings, 0);
    }
    #[test]
    fn legacy_finding_still_deserializes() {
        let f: Finding = serde_json::from_value(
            json!({"id":"b","agent_id":"a","text":"old","parent":"x","timestamp":"then"}),
        )
        .unwrap();
        assert_eq!(f.parents(), BTreeSet::from(["x".to_owned()]));
        assert!(f.evidence_seqs.is_empty());
    }
}

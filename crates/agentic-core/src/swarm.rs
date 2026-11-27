//! Versioned, declarative agent specializations. Profiles describe work, never
//! grant permissions or certify expertise; the host still enforces its tools.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfile {
    pub agent_id: String,
    pub specialty: String,
    pub method: String,
    pub deliverable: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub scope: String,
}
impl AgentProfile {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.agent_id.is_empty() && self.agent_id.len() <= 96,
            "Invalid agent identity"
        );
        for text in [&self.specialty, &self.method, &self.deliverable] {
            ensure!(
                !text.trim().is_empty() && text.len() <= 2000,
                "Profile fields require 1..2000 bytes"
            );
        }
        ensure!(
            self.scope.len() <= 2000 && self.capabilities.len() <= 12,
            "Profile too large"
        );
        ensure!(
            self.capabilities
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 80),
            "Invalid capability"
        );
        Ok(())
    }
}

/// References are validated by the host's provenance store, not by the model.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceLink {
    pub source_id: String,
    pub quote: String,
    pub relation: EvidenceRelation,
    pub explanation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRelation {
    Supports,
    Contradicts,
    Qualifies,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchClaim {
    pub statement: String,
    pub scope: String,
    pub alternative: String,
    pub test: String,
    pub evidence: Vec<EvidenceLink>,
}
impl ResearchClaim {
    pub fn validate(&self) -> Result<()> {
        for s in [&self.statement, &self.scope, &self.alternative, &self.test] {
            ensure!(
                !s.trim().is_empty() && s.len() <= 2000,
                "Claim, scope, alternative and test are required (2000 bytes max)"
            );
        }
        ensure!(
            !self.evidence.is_empty() && self.evidence.len() <= 16,
            "1..16 evidence links required"
        );
        for e in &self.evidence {
            ensure!(
                !e.source_id.is_empty()
                    && e.source_id.len() <= 96
                    && !e.quote.trim().is_empty()
                    && e.quote.len() <= 1800
                    && !e.explanation.trim().is_empty()
                    && e.explanation.len() <= 2000,
                "Invalid evidence link"
            );
        }
        Ok(())
    }
}

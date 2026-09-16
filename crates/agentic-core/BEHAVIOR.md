# Behavioral traces (v2)

`dasein_agentic_core::behavior` provides serializable `Event` and `Finding` types and a read-only `analyze(events, findings)` function returning metrics, links and per-agent trajectories. It does not perform inference or call an LLM.

Findings retain the v1 `parent` field and add `parent_ids`, `evidence_seqs`, and an optional task-specific numeric `answer`. A finding is a claim. Its referenced observations are supporting material, not proof of correctness. The application must enforce experiment isolation, chronology and source availability before accepting references.

New producers use `SCHEMA_VERSION = 2`. The analyzer has its own `ANALYZER_VERSION`. Legacy fields deserialize with defaults. `Event.seq` is monotonically increasing within one experiment and serves as an evidence reference.

Recognized events include `finding.created` (legacy `memory.published`), `memory.read` with `finding_ids`, `strategy.created`/`strategy.changed`, and `finding.validated` with `finding_id`, `valid`, `source = trusted_checker`, a checker version and an explicit scope. Only a trusted controller may emit checker events. Agent declarations must never be mapped directly to those events.

Metrics distinguish unique cross-agent exposures, declared derivations, multi-parent combinations, and successful/failed checker outcomes. Duplicate reads by the same agent do not inflate exposure counts. A read after publication does not substantiate prior exposure. Legacy derivations without a documented prior read yield warnings. A claimed confirmation is not a trusted validation.

`Report.links` uses typed node prefixes `agent:`, `finding:` and `event:`. Each link includes its evidence sequence when available. `trajectories` reference immutable source events instead of replacing them. Reports represent their input trace prefix (`through_seq`) and are deterministic for identical inputs.

This is descriptive analysis, not causal attribution, a novelty detector, or access to hidden reasoning. Checkers are external to this module and must state what they validate. For example, the local document laboratory checks a numeric answer, not the correctness of every supporting argument.

Tests:

```sh
cargo test -p dasein-agentic-core --lib behavior::tests
```

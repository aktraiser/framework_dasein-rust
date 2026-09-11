# Agentic Gateway — local beta

The qualified profile exposes authenticated LLM endpoints and Python Docker Boxes. Follow the [local launch guide](../docs/BETA_LOCAL.md) and [capability/limitations matrix](../docs/SDK_STATUS.md).

- OpenAI chat, embeddings and SSE streaming; configured model routes and ordinary-chat failover.
- SQLite-backed Box ownership and policy, with an ephemeral filesystem per execution.
- Python execution with bounded CPU, memory, time, output and concurrency.
- Operator-only shared infrastructure routes; API keys can access their own Boxes.
- Dashboard provider credentials encrypted with `GATEWAY_MASTER_KEY`.

Set `GATEWAY_CONFIG=beta-local.toml` for localhost without an LLM provider. An explicit provider configuration may use a whole-value `${ENV_VAR}` reference for `api_key`. The named variable must exist when its provider is enabled. Never commit real credentials. `GATEWAY_API_KEY` authenticates the operator; `GATEWAY_ADMIN_PASSWORD` controls dashboard login.

```bash
cargo run --locked -p gateway-server
```

Firecracker execution, other Box runtimes, Anthropic streaming, global spending limits and multi-process Box leases are not qualified or enabled by this profile. It is not a production readiness or latency claim. LLM tests use local mock HTTP servers and incur no provider usage.

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
# Requires Docker and python:3.12-alpine; all services must be disposable:
cargo test --locked --workspace --lib --tests -- --include-ignored --test-threads=1
```

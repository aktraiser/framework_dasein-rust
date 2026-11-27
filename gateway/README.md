# Agentic Gateway — local beta

The qualified profile exposes authenticated LLM endpoints and Python Docker Boxes. Follow the [local launch guide](../docs/BETA_LOCAL.md) and [capability/limitations matrix](../docs/SDK_STATUS.md).

- OpenAI chat, embeddings and SSE streaming; configured model routes and ordinary-chat failover.
- SQLite-backed Box ownership and policy, with an ephemeral filesystem per execution.
- Python execution with bounded CPU, memory, time, output and concurrency.
- Operator-only shared infrastructure routes; API keys can access their own Boxes.
- Dashboard provider credentials encrypted with `GATEWAY_MASTER_KEY`.

Set `GATEWAY_CONFIG=beta-local.toml` for localhost without an LLM provider. An explicit provider configuration may use a whole-value `${ENV_VAR}` reference for `api_key`. The named variable must exist when its provider is enabled. Never commit real credentials. `GATEWAY_API_KEY` authenticates the operator; `GATEWAY_ADMIN_PASSWORD` controls dashboard login.

Dashboard and key protections:

- The session cookie is `HttpOnly` and `SameSite=Strict`. It is also `Secure` whenever the dashboard is used over HTTPS (the browser's `Origin` or a trusted proxy says so), and stays usable on `http://localhost`. `GATEWAY_COOKIE_SECURE=true` or `false` forces the choice.
- State-changing dashboard requests coming from another site are refused.
- After 5 failed logins from one client, logins from it are refused for 5 minutes. A client is an IPv4 address or an IPv6 /64. Whatever the addresses, at most 300 failed logins per 5 minutes are accepted in total; while that is exceeded nobody can log in, which bounds brute force at the price of locking the administrator out during an attack.
- An API key's requests-per-minute and tokens-per-minute limits are enforced on `/v1/chat/completions` and `/v1/embeddings`; an exhausted key gets `429` with `Retry-After`. Counters are kept in the database per clock minute, so they survive a restart and are shared by every process using that database.
- Each LLM call is written to the usage log with its key, provider, model, token counts and cost. Streamed answers are metered too: the gateway asks the provider for the usage of the stream. When no figures arrive (the client left early, or the provider does not report them) they are estimated at four characters per token, so a stream is never free.
- Token limits are checked when a request arrives, with the tokens known so far: requests running in parallel can together exceed the limit before it takes effect, and a key may use its allowance at the end of one clock minute and again at the start of the next.
- Cost is computed from the prices configured for the model (`input_cost_per_1k` and `output_cost_per_1k`, in USD per 1,000 tokens). A model without configured prices is logged at zero. After a failover the answer is priced with the requested model's prices, whichever provider served it.
- Calls to a provider are retried only when it could not be reached, was overloaded (5xx) or limited the rate. A call that timed out after being sent is not replayed, since it may already have been billed.

Network exposure:

- The server listens on `127.0.0.1` unless `host` says otherwise. Set `host = "0.0.0.0"` (or `GATEWAY_HOST`) deliberately, for example inside a container.
- Behind a reverse proxy, list the proxy's address or CIDR range in `trusted_proxies` (or `GATEWAY_TRUSTED_PROXIES`, comma-separated). Its `X-Forwarded-For`, `X-Forwarded-Proto` and `X-Forwarded-Host` headers are then used for login throttling, the cookie's `Secure` flag and the same-site check of dashboard forms. From any other address these headers are ignored.

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

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
- A request reserves its expected tokens when it arrives (its prompt at four characters per token, plus `max_tokens` or 1,024), so requests running in parallel count against each other at once. The reservation is replaced by the real usage when the answer arrives, and given back if the request fails. Limits are counted per clock minute: a key may use its allowance at the end of one minute and again at the start of the next.
- Cost is computed from the prices configured for the model (`input_cost_per_1k` and `output_cost_per_1k`, in USD per 1,000 tokens). A model without configured prices is logged at zero. After a failover the answer is priced with the requested model's prices, whichever provider served it.
- Calls to a provider are retried only when it could not be reached, was overloaded (5xx) or limited the rate. A call that timed out after being sent is not replayed, since it may already have been billed.

Network exposure:

- The server listens on `127.0.0.1` unless `host` says otherwise. Set `host = "0.0.0.0"` (or `GATEWAY_HOST`) deliberately, for example inside a container.
- Behind a reverse proxy, list the proxy's address or CIDR range in `trusted_proxies` (or `GATEWAY_TRUSTED_PROXIES`, comma-separated). Its `X-Forwarded-For`, `X-Forwarded-Proto` and `X-Forwarded-Host` headers are then used for login throttling, the cookie's `Secure` flag and the same-site check of dashboard forms. From any other address these headers are ignored.

```bash
cargo run --locked -p gateway-server
```

## Several processes on one machine

Several gateway processes can serve from the same SQLite file, each on its own port behind a reverse proxy or load balancer of your choice. Give them the same `DATABASE_URL`, `GATEWAY_MASTER_KEY`, `GATEWAY_ADMIN_PASSWORD` and configuration. A process that crashes or restarts does not affect the others, and what must be agreed on is kept in the database:

- API keys, providers, dashboard sessions and the usage log, as before. A provider changed in the dashboard of one process is used by all of them at the next request.
- A key's request and token limits: the processes count against the same per-minute counters.
- Failed dashboard logins: five failures block a client whichever processes took them.
- Boxes: one execution at a time per box and at most 16 boxes executing at once, over all processes. A box deleted through one process stops executing in another within about half a second. An execution whose process dies gives its place back when its timeout, plus 30 seconds, has passed.

The database is opened in WAL mode and a process waits up to ten seconds for another one's write. Processes started together on a new file take turns creating it.

Tried with three processes on one file: started together, a session made on one accepted by the others, 150 calls sent at once over the three with a key limited to 30 a minute (30 let through, 120 refused), one process killed with `kill -9` and restarted while the others served, and wrong passwords spread over the three (the sixth attempt refused).

This is not high availability across machines: the SQLite file and the Docker daemon are on one host, and losing that host loses the service. `/metrics` reports each process on its own. No load balancer is provided.

Firecracker execution, other Box runtimes, Anthropic streaming and global spending limits are not qualified or enabled by this profile. It is not a production readiness or latency claim. LLM tests use local mock HTTP servers and incur no provider usage.

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
# Requires Docker and python:3.12-alpine; all services must be disposable:
cargo test --locked --workspace --lib --tests -- --include-ignored --test-threads=1
```

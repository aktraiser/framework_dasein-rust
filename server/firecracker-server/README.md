# Authenticated Firecracker HTTP adapter

This service calls the SDK's real Linux/KVM backend. Prepare the [guest image and KVM host](../firecracker-guest/README.md) first. It refuses to start without a nonempty `API_KEY` or missing backend prerequisites. The legacy `server/agent.py` host-shell service is retired and exits immediately.

```sh
export API_KEY="$(openssl rand -hex 32)"
export FIRECRACKER_BIN=/path/to/firecracker
export KERNEL_PATH=/path/to/vmlinux
export ROOTFS_PATH=/path/to/rootfs.ext4
export WORKSPACE=/tmp/dasein-fc
export BIND=127.0.0.1:8080
cargo run --manifest-path server/firecracker-server/Cargo.toml --locked
```

Keep the service on loopback for a local lab. For access from a Mac to its Linux VM, use a local SSH tunnel. Public access requires an authenticated TLS deployment and the production host hardening described in the guest guide. `PORT` is supported when `BIND` is omitted; the default listener is `127.0.0.1:8080`.

Both `GET /health` and `POST /execute` require `X-API-Key`. Health reports prerequisite availability, not a cached guest-boot test. Execution accepts:

```json
{"code":"print('hello')","language":"python","timeout_ms":30000}
```

Supported languages: `python`, `python3`, `sh`, `shell`; omitting language treats code as a guest shell command. Response fields: `success`, `exit_code`, `stdout`, `stderr`, `execution_time_ms`. Files are ephemeral; artifact extraction is not implemented. Limits: 256 KiB source (including the shell wrapper), 2 MiB HTTP body, 4 simultaneous VMs, 512 MiB and 2 vCPUs per VM by default, 1 MiB output, 1–120000 ms total deadline including boot. Unsupported languages and invalid limits return 400; bad credentials 401; capacity 429; unavailable backend 503; timeout 408; other VM failures 502. No automatic request retry occurs in the SDK client by default.

Use `RemoteSandbox::builder(url).api_key(key).language("python").build()` with the SDK's `remote` feature. `GatewaySandbox` targets the separate gateway API and is not interchangeable with this adapter.

```sh
cargo test --manifest-path server/firecracker-server/Cargo.toml --locked
cargo clippy --manifest-path server/firecracker-server/Cargo.toml --all-targets -- -D warnings
```

The repository's real KVM test is documented in the guest guide. The local qualification also exercised this HTTP adapter with real guest Python, authentication, timeout mapping and abrupt server death. Logs and exact tested asset hashes are recorded in the delivery report; no external server or paid service was provisioned.

#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${DASEIN_TEST_NATS_URL:=nats://127.0.0.1:4222}"
: "${DASEIN_TEST_REDIS_URL:=redis://127.0.0.1:6379/}"
export DASEIN_TEST_NATS_URL DASEIN_TEST_REDIS_URL
# Dedicated disposable services only: tests create/delete streams and buckets.
cargo test --locked --workspace --all-features --lib --tests -- --include-ignored --test-threads=1
# Ignored documentation examples are pseudocode, not integration tests.
cargo test --locked --workspace --all-features --doc

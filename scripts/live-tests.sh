#!/usr/bin/env bash
# Run the checks against real services. Each one skips itself unless its
# variables are set; see docs/LIVE_TESTS.md.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --locked \
  -p dasein-agentic-llm -p dasein-agentic-mcp -p dasein-agentic-chat-platforms \
  --all-features --test live -- --include-ignored --nocapture --test-threads=1

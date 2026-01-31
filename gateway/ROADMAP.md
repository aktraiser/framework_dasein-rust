# Agentic Gateway - Unified LLM & Sandbox Gateway

> **Vision**: Un gateway unifié en Rust pour l'accès LLM multi-provider et l'exécution de code sécurisée via Firecracker microVMs.

---

## Recherche & Inspiration

### LiteLLM - AI Gateway de Référence

[LiteLLM](https://github.com/BerriAI/litellm) est le standard de facto pour les gateways LLM:

| Feature | Description |
|---------|-------------|
| **100+ Providers** | OpenAI, Anthropic, Google, Azure, Bedrock, Cohere, HuggingFace, VLLM... |
| **OpenAI-Compatible** | API unifiée format OpenAI pour tous les providers |
| **Load Balancing** | Round-robin, least-connections, weighted routing |
| **Fallback** | Retry automatique sur provider alternatif si erreur |
| **Rate Limiting** | Per-user, per-team, per-model limits (tokens/requests) |
| **Cost Tracking** | Suivi des coûts par provider/model/user |
| **Virtual Keys** | API keys virtuelles avec budgets et permissions |
| **Caching** | Cache sémantique pour réduire les appels |
| **Guardrails** | Filtrage content, PII detection, prompt injection |

**Performance**: 8ms P95 latency @ 1k RPS

**Architecture**:
```
┌─────────────────────────────────────────────────────────────────┐
│                      LiteLLM Proxy                              │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐        │
│  │  Router  │→ │ Auth/RBAC│→ │Rate Limit│→ │  Cache   │        │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘        │
│        │                                                        │
│        ▼                                                        │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │               Provider Adapters                          │  │
│  │  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ │  │
│  │  │ OpenAI │ │Anthropic│ │ Gemini │ │ Azure  │ │ VLLM   │ │  │
│  │  └────────┘ └────────┘ └────────┘ └────────┘ └────────┘ │  │
│  └──────────────────────────────────────────────────────────┘  │
│        │                                                        │
│        ▼                                                        │
│  ┌──────────┐  ┌──────────┐                                    │
│  │ Postgres │  │  Redis   │  (Config, Keys, Rate tracking)     │
│  └──────────┘  └──────────┘                                    │
└─────────────────────────────────────────────────────────────────┘
```

Sources:
- [LiteLLM Documentation](https://docs.litellm.ai/docs/simple_proxy)
- [Top LLM Gateways 2025](https://agenta.ai/blog/top-llm-gateways)
- [LiteLLM Alternatives](https://www.pomerium.com/blog/litellm-alternatives)

---

### E2B - Code Execution Platform

[E2B](https://github.com/e2b-dev/E2B) est la référence pour l'exécution de code AI:

| Feature | Description |
|---------|-------------|
| **Firecracker microVMs** | Isolation hardware-level, boot < 200ms |
| **Multi-language** | Python, JavaScript, TypeScript, Bash... |
| **Jupyter Kernel** | Code interpreter intégré |
| **Filesystem** | Système de fichiers isolé par sandbox |
| **Networking** | Accès réseau contrôlé |
| **Stateful** | Sessions persistantes |

**Architecture E2B**:
```
┌─────────────────────────────────────────────────────────────────┐
│                        E2B Platform                             │
│                                                                 │
│  ┌──────────┐     ┌──────────────────────────────────────────┐ │
│  │   API    │────▶│           Sandbox Orchestrator           │ │
│  │  Server  │     └──────────────────────────────────────────┘ │
│  └──────────┘                      │                           │
│                    ┌───────────────┼───────────────┐           │
│                    ▼               ▼               ▼           │
│              ┌──────────┐   ┌──────────┐   ┌──────────┐       │
│              │ Sandbox  │   │ Sandbox  │   │ Sandbox  │       │
│              │ (microVM)│   │ (microVM)│   │ (microVM)│       │
│              │┌────────┐│   │┌────────┐│   │┌────────┐│       │
│              ││Jupyter ││   ││Jupyter ││   ││Jupyter ││       │
│              │└────────┘│   │└────────┘│   │└────────┘│       │
│              └──────────┘   └──────────┘   └──────────┘       │
│                    │               │               │           │
│              ┌─────┴───────────────┴───────────────┴─────┐    │
│              │              Firecracker Host              │    │
│              └───────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

Sources:
- [E2B Documentation](https://e2b.dev/docs)
- [E2B Code Interpreter](https://github.com/e2b-dev/code-interpreter)
- [Firecracker vs QEMU](https://e2b.dev/blog/firecracker-vs-qemu)

---

### Firecracker - microVM Technology

[Firecracker](https://github.com/firecracker-microvm/firecracker) par AWS:

| Métrique | Valeur |
|----------|--------|
| **Boot time** | < 125ms |
| **Memory footprint** | < 5 MiB |
| **Creation rate** | 150 microVMs/sec/host |
| **Security** | KVM + Jailer (cgroups/namespaces) |

**API REST** exposée en OpenAPI format:
- `PUT /machine-config` - Configure vCPUs, memory
- `PUT /boot-source` - Set kernel, initrd
- `PUT /drives/{id}` - Attach block devices
- `PUT /network-interfaces/{id}` - Configure networking
- `PUT /actions` - Start/stop VM
- `GET /` - Get full VM state

Sources:
- [Firecracker GitHub](https://github.com/firecracker-microvm/firecracker)
- [Northflank Secure Runtime](https://northflank.com/blog/secure-runtime-for-codegen-tools-microvms-sandboxing-and-execution-at-scale)
- [MicroVMs for AI Agents](https://glama.ai/blog/2025-07-25-micro-vms-over-containers-a-safer-execution-path-for-ai-agents)

---

### Rust Gateway Implementations

Pourquoi Rust pour un gateway? ([Source](https://blog.langdb.ai/why-we-built-an-ai-gateway-in-rust-a-performance-centric-decision))

| Aspect | Python | Rust |
|--------|--------|------|
| **Concurrency** | GIL bottleneck | Native async (Tokio) |
| **Memory** | GC pauses | Zero-cost abstractions |
| **Latency** | ~50ms overhead | < 1ms overhead |
| **Safety** | Runtime errors | Compile-time guarantees |

**Projets Rust existants**:
- [agentgateway](https://blog.mygraphql.com/en/posts/ai/ai-devops/agent-gateway/agentgateway-impl/) - MCP/A2A gateway
- [llm-gateway crate](https://crates.io/crates/llm-gateway) - Multi-provider proxy
- [tokio-quiche](https://www.infoq.com/news/2025/12/quic-http3-rust/) - QUIC/HTTP3 by Cloudflare

---

## Architecture Proposée: Agentic Gateway

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          AGENTIC GATEWAY (Rust)                             │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                         HTTP/gRPC Server (Axum)                      │   │
│  │  /v1/chat/completions  /v1/embeddings  /v1/execute  /v1/sandbox     │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                    │                                        │
│  ┌─────────────────────────────────┼─────────────────────────────────┐     │
│  │                         MIDDLEWARE LAYER                          │     │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐│     │
│  │  │   Auth   │ │Rate Limit│ │  Cache   │ │ Logging  │ │ Metrics  ││     │
│  │  │  (JWT)   │ │(Token/Req)│ │(Semantic)│ │(Tracing) │ │(Prometheus)│    │
│  │  └──────────┘ └──────────┘ └──────────┘ └──────────┘ └──────────┘│     │
│  └───────────────────────────────────────────────────────────────────┘     │
│                                    │                                        │
│         ┌──────────────────────────┴──────────────────────────┐            │
│         │                                                      │            │
│         ▼                                                      ▼            │
│  ┌─────────────────────────────────┐    ┌─────────────────────────────────┐│
│  │        LLM GATEWAY              │    │       SANDBOX GATEWAY           ││
│  │  ┌───────────────────────────┐  │    │  ┌───────────────────────────┐  ││
│  │  │      Router/Balancer      │  │    │  │    Sandbox Orchestrator   │  ││
│  │  │  (Fallback, Retry, LB)    │  │    │  │  (Pool, Lifecycle, Scale) │  ││
│  │  └───────────────────────────┘  │    │  └───────────────────────────┘  ││
│  │            │                    │    │              │                  ││
│  │  ┌─────────┴─────────┐          │    │  ┌───────────┴───────────┐      ││
│  │  ▼         ▼         ▼          │    │  ▼           ▼           ▼      ││
│  │┌──────┐ ┌──────┐ ┌──────┐       │    │┌─────────┐┌─────────┐┌─────────┐││
│  ││OpenAI│ │Claude│ │Gemini│ ...   │    ││Firecracker││gVisor  ││ Docker │││
│  │└──────┘ └──────┘ └──────┘       │    ││ microVM   ││Sandbox ││Container│││
│  │                                 │    │└─────────┘└─────────┘└─────────┘││
│  └─────────────────────────────────┘    └─────────────────────────────────┘│
│                                    │                                        │
│  ┌─────────────────────────────────┴─────────────────────────────────┐     │
│  │                         PERSISTENCE LAYER                          │     │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐              │     │
│  │  │ Postgres │ │  Redis   │ │   NATS   │ │  S3/Minio│              │     │
│  │  │ (Config) │ │ (Cache)  │ │ (Events) │ │(Artifacts)│             │     │
│  │  └──────────┘ └──────────┘ └──────────┘ └──────────┘              │     │
│  └───────────────────────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Feuille de Route

### Phase 1: Foundation (2 semaines)

#### 1.1 Project Setup
- [ ] Cargo workspace structure
- [ ] CI/CD (GitHub Actions)
- [ ] Docker/docker-compose dev environment
- [ ] Configuration management (YAML + env)

#### 1.2 Core HTTP Server
- [ ] Axum server with graceful shutdown
- [ ] Health check endpoints (`/health`, `/ready`)
- [ ] OpenAPI spec generation
- [ ] Request/Response logging (tracing)

#### 1.3 Basic Auth
- [ ] API key validation
- [ ] Virtual keys avec metadata
- [ ] JWT support (optional)

**Livrables Phase 1**:
```bash
# Server démarre et répond
curl http://localhost:8080/health
{"status": "ok", "version": "0.1.0"}

# Auth fonctionne
curl -H "Authorization: Bearer sk-xxx" http://localhost:8080/v1/models
{"models": [...]}
```

---

### Phase 2: LLM Gateway Core (3 semaines)

#### 2.1 Provider Adapters
- [ ] Trait `LLMProvider` avec async streaming
- [ ] OpenAI adapter (reference implementation)
- [ ] Anthropic adapter (Claude)
- [ ] Google adapter (Gemini)
- [ ] OpenAI-compatible adapter (VLLM, Ollama, etc.)

```rust
#[async_trait]
pub trait LLMProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, ProviderError>;

    fn chat_completion_stream(
        &self,
        request: ChatCompletionRequest,
    ) -> impl Stream<Item = Result<ChatCompletionChunk, ProviderError>>;

    async fn embeddings(
        &self,
        request: EmbeddingRequest,
    ) -> Result<EmbeddingResponse, ProviderError>;
}
```

#### 2.2 Router & Load Balancer
- [ ] Model → Provider(s) mapping
- [ ] Load balancing strategies:
  - Round-robin
  - Least-connections
  - Weighted random
  - Latency-based
- [ ] Health checking des providers

#### 2.3 Fallback & Retry
- [ ] Retry with exponential backoff
- [ ] Fallback chain (primary → secondary → ...)
- [ ] Circuit breaker pattern
- [ ] Cooldown tracking (Redis)

**Livrables Phase 2**:
```bash
# Chat completion avec fallback automatique
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer sk-xxx" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}]}'

# Streaming
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer sk-xxx" \
  -d '{"model": "claude-3", "messages": [...], "stream": true}'
```

---

### Phase 3: Rate Limiting & Cost Tracking (2 semaines)

#### 3.1 Rate Limiting
- [ ] Token-based rate limiting (pas juste requests)
- [ ] Per-key, per-model, per-team limits
- [ ] Sliding window algorithm
- [ ] Redis-backed distributed rate limiting

```rust
pub struct RateLimitConfig {
    pub requests_per_minute: Option<u32>,
    pub tokens_per_minute: Option<u32>,
    pub tokens_per_day: Option<u32>,
}
```

#### 3.2 Cost Tracking
- [ ] Token counting per request
- [ ] Cost calculation par provider/model
- [ ] Usage aggregation (hourly, daily, monthly)
- [ ] Budget alerts et hard limits

#### 3.3 Virtual Keys
- [ ] Key generation avec metadata
- [ ] Budget assignment par key
- [ ] Key rotation
- [ ] Audit log

**Livrables Phase 3**:
```bash
# Rate limit info dans headers
curl -I http://localhost:8080/v1/chat/completions
X-RateLimit-Remaining-Requests: 95
X-RateLimit-Remaining-Tokens: 9500

# Usage stats
curl http://localhost:8080/v1/usage?key=sk-xxx
{"tokens_used": 5000, "cost_usd": 0.15, "requests": 50}
```

---

### Phase 4: Sandbox Gateway - Firecracker (4 semaines)

#### 4.1 Firecracker Integration
- [ ] Firecracker binary management
- [ ] microVM lifecycle (create, start, stop, destroy)
- [ ] Kernel & rootfs management
- [ ] Jailer integration (security)

#### 4.2 Sandbox Pool
- [ ] Pre-warmed sandbox pool
- [ ] Dynamic scaling (min/max)
- [ ] Sandbox health monitoring
- [ ] Garbage collection (idle timeout)

```rust
pub struct SandboxPool {
    config: SandboxPoolConfig,
    available: Arc<Mutex<Vec<Sandbox>>>,
    in_use: Arc<Mutex<HashMap<SandboxId, Sandbox>>>,
}

pub struct SandboxPoolConfig {
    pub min_ready: usize,      // Minimum sandboxes prêts
    pub max_total: usize,      // Maximum total
    pub idle_timeout: Duration, // Timeout avant destruction
    pub boot_timeout: Duration, // Timeout de boot
}
```

#### 4.3 Code Execution API
- [ ] `/v1/sandbox/create` - Créer une sandbox
- [ ] `/v1/sandbox/{id}/execute` - Exécuter du code
- [ ] `/v1/sandbox/{id}/files` - Upload/download files
- [ ] `/v1/sandbox/{id}/destroy` - Détruire la sandbox
- [ ] WebSocket pour sessions interactives

#### 4.4 Multi-runtime Support
- [ ] Python (avec pip packages)
- [ ] Node.js/TypeScript (avec npm)
- [ ] Rust (cargo)
- [ ] Go

**Livrables Phase 4**:
```bash
# Créer sandbox
curl -X POST http://localhost:8080/v1/sandbox/create \
  -d '{"runtime": "python", "packages": ["numpy", "pandas"]}'
{"sandbox_id": "sb-xxx", "status": "ready"}

# Exécuter code
curl -X POST http://localhost:8080/v1/sandbox/sb-xxx/execute \
  -d '{"code": "print(sum([1,2,3,4]))", "timeout": 30}'
{"output": "10\n", "exit_code": 0, "duration_ms": 45}
```

---

### Phase 5: Integration & Enterprise Features (3 semaines)

#### 5.1 Observability
- [ ] Prometheus metrics endpoint
- [ ] Distributed tracing (OpenTelemetry)
- [ ] Structured logging (JSON)
- [ ] Grafana dashboards

#### 5.2 Caching
- [ ] Semantic caching (embeddings similarity)
- [ ] Exact match caching
- [ ] Cache invalidation strategies
- [ ] Redis cluster support

#### 5.3 Guardrails
- [ ] Input validation (max tokens, content filter)
- [ ] Output validation
- [ ] PII detection (optional)
- [ ] Prompt injection detection

#### 5.4 Admin UI (Optional)
- [ ] Dashboard web (React/Vue)
- [ ] Key management UI
- [ ] Usage analytics
- [ ] Real-time monitoring

---

### Phase 6: Production Hardening (2 semaines)

#### 6.1 High Availability
- [ ] Stateless design (all state in Redis/Postgres)
- [ ] Kubernetes deployment manifests
- [ ] Horizontal Pod Autoscaler
- [ ] PodDisruptionBudget

#### 6.2 Security
- [ ] TLS termination
- [ ] mTLS for internal services
- [ ] Secret management (Vault integration)
- [ ] Security audit

#### 6.3 Performance
- [ ] Connection pooling
- [ ] Request batching
- [ ] Compression (gzip, brotli)
- [ ] Benchmark suite

---

## Stack Technique

| Composant | Technologie | Justification |
|-----------|-------------|---------------|
| **Runtime** | Rust + Tokio | Performance, safety, async natif |
| **HTTP Server** | Axum | Modern, performant, tower middleware |
| **Serialization** | serde + serde_json | Standard Rust |
| **HTTP Client** | reqwest | Async, connection pooling |
| **Database** | SQLx + Postgres | Async, compile-time queries |
| **Cache** | Redis (deadpool-redis) | Distributed, fast |
| **Config** | config-rs | YAML + env layering |
| **Tracing** | tracing + tracing-subscriber | Structured logging |
| **Metrics** | prometheus | Standard metrics format |
| **CLI** | clap | Args parsing |
| **Testing** | tokio-test + wiremock | Async tests, HTTP mocking |

---

## Structure du Projet

```
gateway/
├── Cargo.toml              # Workspace root
├── config/
│   ├── default.yaml        # Default config
│   ├── development.yaml    # Dev overrides
│   └── production.yaml     # Prod overrides
├── crates/
│   ├── gateway-core/       # Core types, traits
│   ├── gateway-llm/        # LLM providers
│   ├── gateway-sandbox/    # Firecracker integration
│   ├── gateway-server/     # HTTP server, routes
│   └── gateway-cli/        # CLI binary
├── docker/
│   ├── Dockerfile
│   └── docker-compose.yml
├── k8s/
│   ├── deployment.yaml
│   └── service.yaml
└── tests/
    ├── integration/
    └── e2e/
```

---

## Métriques de Succès

| Métrique | Target | Mesure |
|----------|--------|--------|
| **Latency P50** | < 5ms overhead | Prometheus histogram |
| **Latency P99** | < 20ms overhead | Prometheus histogram |
| **Throughput** | > 10k req/s | Load testing |
| **Sandbox boot** | < 200ms | Timing logs |
| **Availability** | 99.9% | Uptime monitoring |
| **Error rate** | < 0.1% | Error counters |

---

## Références

### Documentation
- [LiteLLM Docs](https://docs.litellm.ai/)
- [E2B Docs](https://e2b.dev/docs)
- [Firecracker Docs](https://github.com/firecracker-microvm/firecracker/blob/main/docs/getting-started.md)
- [Axum Examples](https://github.com/tokio-rs/axum/tree/main/examples)

### Articles
- [Why Rust for AI Gateway](https://blog.langdb.ai/why-we-built-an-ai-gateway-in-rust-a-performance-centric-decision)
- [Rate Limiting in LLM Gateway](https://www.truefoundry.com/blog/rate-limiting-in-llm-gateway)
- [API Gateway for LLM Requests](https://api7.ai/learning-center/api-gateway-guide/api-gateway-proxy-llm-requests)
- [agentgateway Implementation](https://blog.mygraphql.com/en/posts/ai/ai-devops/agent-gateway/agentgateway-impl/)

### Projets Open Source
- [LiteLLM](https://github.com/BerriAI/litellm) - Python reference
- [E2B](https://github.com/e2b-dev/E2B) - Sandbox reference
- [Firecracker](https://github.com/firecracker-microvm/firecracker)
- [llm-gateway crate](https://crates.io/crates/llm-gateway)
- [agentgateway](https://github.com/agentgateway/agentgateway)

---

## Timeline Estimée

| Phase | Durée | Cumul |
|-------|-------|-------|
| Phase 1: Foundation | 2 sem | 2 sem |
| Phase 2: LLM Gateway | 3 sem | 5 sem |
| Phase 3: Rate Limiting | 2 sem | 7 sem |
| Phase 4: Sandbox | 4 sem | 11 sem |
| Phase 5: Enterprise | 3 sem | 14 sem |
| Phase 6: Production | 2 sem | 16 sem |

**Total: ~4 mois** pour une v1.0 production-ready.

---

## Quick Start (Future)

```bash
# Clone et build
git clone https://github.com/your-org/agentic-gateway
cd agentic-gateway
cargo build --release

# Configuration
cp config/default.yaml config/local.yaml
# Edit config/local.yaml with your API keys

# Run
./target/release/gateway-server --config config/local.yaml

# Test
curl http://localhost:8080/health
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello!"}]}'
```

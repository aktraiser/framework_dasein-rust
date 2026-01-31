# Documentation Agentic-RS

> Le framework Rust pour réseaux d'agents IA distribués à grande échelle

**Notre niche:** 100+ agents concurrents, là où Python s'effondre.

---

## Index

| Document | Description |
|----------|-------------|
| [GETTING_STARTED.md](./GETTING_STARTED.md) | **Guide de démarrage rapide** (commencez ici!) |
| [FEATURES.md](./FEATURES.md) | Guide des features Cargo (LLM, Sandbox, Storage) |
| [VISION.md](./VISION.md) | Positionnement stratégique et roadmap produit |
| [ARCHITECTURE.md](./ARCHITECTURE.md) | Architecture 3 couches (Superviseur/Exécuteur/Validateur) |
| [MULTI_EXECUTOR.md](./MULTI_EXECUTOR.md) | Progressive Locking Pipeline avec Error Fingerprinting |
| [AGENT_CREATION_GUIDE.md](./AGENT_CREATION_GUIDE.md) | Guide pratique pour créer et dupliquer des agents |
| [NATS_PROTOCOL.md](./NATS_PROTOCOL.md) | Spécification du protocole NATS (subjects, messages) |
| [CRITICAL_ANALYSIS.md](./CRITICAL_ANALYSIS.md) | Analyse technique, risques et roadmap |

---

## Quick Start

### Prérequis

```bash
# NATS Cluster (obligatoire)
docker run -d --name nats -p 4222:4222 -p 8222:8222 nats:latest -js

# Vérifier
nats server info

# Variables d'environnement
export GEMINI_API_KEY="your-key"              # Obligatoire
export FAST_MODEL="gemini-2.0-flash"          # Optionnel (défaut)
export SMART_MODEL="gemini-3-flash-preview"   # Optionnel (défaut: gemini-2.0-flash)
export CONTEXT7_API_KEY="your-key"            # Optionnel (active MCP Doc)
```

### Exemple: Réseau Multi-Superviseurs

```rust
use agentic_rs::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Connexion NATS
    let bus = NatsBus::connect("nats://localhost:4222").await?;

    // Superviseur CODE avec 4 exécuteurs
    let code_sup = Supervisor::builder()
        .id("sup-code")
        .domain("code")
        .executors(4)
        .llm_gemini("gemini-2.0-flash")
        .sandbox_process()
        .allow_lending(true)      // Peut prêter ses exécuteurs
        .allow_borrowing(true)    // Peut emprunter
        .build(bus.clone()).await?;

    // Superviseur DATA avec 2 exécuteurs
    let data_sup = Supervisor::builder()
        .id("sup-data")
        .domain("data")
        .executors(2)
        .llm_gemini("gemini-2.0-flash")
        .allow_lending(true)
        .build(bus.clone()).await?;

    // Démarrer les superviseurs
    code_sup.start().await?;
    data_sup.start().await?;

    // Les superviseurs communiquent via NATS
    // Allocation dynamique automatique si surcharge

    // Envoyer une tâche
    let result = code_sup.execute(Task::new(
        "generate",
        json!({ "task": "Write a REST API" })
    )).await?;

    Ok(())
}
```

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                                                                             │
│                              NATS CLUSTER                                   │
│                              (JetStream)                                    │
│                                   │                                         │
│      ┌────────────────────────────┼────────────────────────────┐           │
│      │                            │                            │           │
│      ▼                            ▼                            ▼           │
│ ┌─────────────┐            ┌─────────────┐            ┌─────────────┐      │
│ │ SUPERVISEUR │◄──────────►│ SUPERVISEUR │◄──────────►│ SUPERVISEUR │      │
│ │    CODE     │  allocate  │    DATA     │  allocate  │   INFRA     │      │
│ └──────┬──────┘            └──────┬──────┘            └──────┬──────┘      │
│        │                          │                          │             │
│   ┌────┴────┐                ┌────┴────┐                ┌────┴────┐       │
│   ▼         ▼                ▼         ▼                ▼         ▼       │
│ [EXE ⚡]  [EXE 🧠]         [EXE]    [EXE]            [EXE]    [EXE]        │
│ Fast    Smart                                                             │
│        │                          │                          │             │
│        ▼                          ▼                          ▼             │
│   [VALIDATOR]               [VALIDATOR]               [VALIDATOR]          │
│   Linter→Clippy             Linter→Clippy             Linter→Clippy       │
│   →Compile→Tests            →Compile→Tests            →Compile→Tests      │
│   →MCP Doc                  →MCP Doc                  →MCP Doc            │
│                                                                             │
│ ════════════════════════════════════════════════════════════════════════   │
│                              audit.>                                        │
│                       (JetStream persisté)                                  │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Fonctionnalités clés du Pipeline

| Fonctionnalité | Description |
|----------------|-------------|
| **Progressive Locking** | Types → Stubs → Logic (verrouille chaque stage) |
| **Error Fingerprinting** | Route erreurs simples → Fast ⚡, complexes → Smart 🧠 |
| **Clippy Integration** | Détecte anti-patterns avant tests |
| **MCP Doc Validator** | Enrichit erreurs avec documentation (Context7) |
| **RollbackManager** | Empêche régressions, retourne au meilleur code |

---

## Pour Qui?

| Client Type | Use Case |
|-------------|----------|
| **Enterprise** | 100+ agents, SLA strict, compliance |
| **Fintech** | Latence critique, audit réglementaire |
| **Télécoms** | Millions de devices, edge computing |
| **Défense** | Sécurité, air-gapped, traçabilité |

**On ne cible PAS:** Prototypes, agents simples, équipes sans infra.

---

## Différenciateurs

| Feature | Agentic-RS | LangChain/CrewAI |
|---------|------------|------------------|
| 100+ agents | ✅ Native | ❌ OOM |
| Latence routing | <5ms | 50-100ms |
| Allocation dynamique | ✅ Unique | ❌ |
| Mémoire/agent | 20MB | 100-200MB |
| Audit enterprise | ✅ JetStream | ❌ |
| Error Fingerprinting | ✅ Fast→Smart | ❌ |
| Progressive Locking | ✅ Types→Stubs→Logic | ❌ |
| Clippy integration | ✅ Anti-patterns | ❌ |
| MCP Documentation | ✅ Context7 | ❌ |

---

## Ressources

- [Exemples](../examples/)
- [API Reference](./API.md) *(à venir)*
- [Changelog](../CHANGELOG.md) *(à venir)*

---

## Support

- GitHub Issues: [github.com/your-org/agentic-rs/issues](https://github.com/your-org/agentic-rs/issues)
- Enterprise Support: contact@agentic-rs.io *(à venir)*

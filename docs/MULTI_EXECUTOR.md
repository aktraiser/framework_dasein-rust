# Multi-Executor Pipeline

## Vue d'ensemble

Le Multi-Executor implémente une approche de **Progressive Locking** pour générer du code Rust valide via LLM. Au lieu de générer tout le code d'un coup et espérer qu'il compile, on décompose en 3 stages et on verrouille chaque stage une fois validé.

**Fonctionnalités clés:**
- **Progressive Locking**: Types → Stubs → Logic (verrouille chaque stage)
- **Error Fingerprinting**: Route simple errors → Fast model, complex → Smart model
- **Clippy Integration**: Détecte les anti-patterns avant les tests
- **MCP Doc Validator**: Enrichit les erreurs avec de la documentation (Context7)
- **RollbackManager**: Empêche les régressions en trackant la meilleure tentative

```
┌─────────────────────────────────────────────────────────────┐
│                    PROGRESSIVE LOCKING                       │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│   [Stage 1: TYPES]  ──lint──►  LOCK  ─┐                     │
│                                        │                     │
│   [Stage 2: STUBS]  ──lint──►  LOCK  ─┼─► Context           │
│        (uses Types)                    │                     │
│                                        │                     │
│   [Stage 3: LOGIC]  ──validate──►  DONE                     │
│        (uses Types+Stubs)                                    │
│        │                                                     │
│        └──► Linter → Clippy → Compile → Tests → MCP Doc     │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

## Architecture

### Composants principaux

| Composant | Rôle | Fichier |
|-----------|------|---------|
| `BusCoordinator` | Coordination NATS JetStream | `bus/coordinator.rs` |
| `StateStore` | KV store pour artefacts validés | `bus/state_store.rs` |
| `BusLinter` | Pré-validation rapide (2-3ms) | `bus/linter.rs` |
| `RollbackManager` | Tracking et rollback des tentatives | `bus/rollback.rs` |
| `ErrorFingerprinter` | Classification des erreurs → sélection du modèle | `bus/error_fingerprint.rs` |
| `SandboxPipelineValidator` | Compilation + Clippy + Tests | `sandbox_validator.rs` |
| `MCPDocValidator` | Enrichissement avec documentation | `mcp_doc_validator.rs` |
| `ValidatorPipeline` | Chaîne de validation complète | `validator.rs` |
| `Executor` | Interface LLM (Gemini, Claude, etc.) | `executor.rs` |

### Flux de données

```
                    ┌──────────────────┐
                    │   Task (prompt)  │
                    └────────┬─────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
              ▼              ▼              ▼
        ┌─────────┐   ┌─────────┐   ┌─────────┐
        │  TYPES  │   │  STUBS  │   │  LOGIC  │
        │  Stage  │──►│  Stage  │──►│  Stage  │
        └────┬────┘   └────┬────┘   └────┬────┘
             │              │              │
             ▼              ▼              ▼
        ┌─────────┐   ┌─────────┐   ┌─────────────────────────┐
        │ Linter  │   │ Linter  │   │ Validation Stack        │
        │  (2ms)  │   │  (2ms)  │   │ 1. Linter (2ms)         │
        └────┬────┘   └────┬────┘   │ 2. Clippy (5s)          │
             │              │        │ 3. Compile (10s)        │
             │              │        │ 4. Tests (15s)          │
             │              │        │ 5. MCP Doc (optional)   │
             │              │        └───────────┬─────────────┘
             │              │                    │
             │              │              ┌─────▼──────┐
             │              │              │ Fingerprint│
             │              │              │  ⚡ Fast   │
             │              │              │  🧠 Smart  │
             │              │              └─────┬──────┘
             ▼              ▼                    ▼
        ┌─────────┐   ┌─────────┐         ┌─────────┐
        │  LOCKED │   │  LOCKED │         │  FINAL  │
        │  types  │   │  stubs  │         │  code   │
        └─────────┘   └─────────┘         └─────────┘
```

## Les 3 Stages

### Stage 1: TYPES

**Objectif:** Générer uniquement les définitions de types.

**Prompt:**
```
Generate ONLY the type definitions for this task:
- use statements (imports)
- struct definitions
- enum definitions
- type aliases

NO function implementations. NO impl blocks.
```

**Validation:** Linter uniquement (syntaxe correcte)

**Output:** Code Rust avec types verrouillés

### Stage 2: STUBS

**Objectif:** Générer les signatures de fonctions avec `todo!()`.

**Prompt:**
```
=== LOCKED TYPES (DO NOT MODIFY) ===
{types from Stage 1}

Generate the impl blocks with function SIGNATURES ONLY.
Use todo!() for all function bodies.
```

**Validation:** Linter uniquement

**Output:** Code Rust avec types + stubs verrouillés

### Stage 3: LOGIC

**Objectif:** Implémenter les corps de fonctions.

**Prompt:**
```
=== LOCKED TYPES AND SIGNATURES (DO NOT MODIFY) ===
{code from Stage 2}

Now implement the function bodies.
Keep all type definitions and function signatures EXACTLY as shown.
```

**Validation:** Linter + Compilation + Tests

**Output:** Code Rust complet et fonctionnel

## Mécanismes clés

### 1. Error Feedback Loop

Quand une tentative échoue, les erreurs sont injectées dans le prompt suivant:

```rust
=== PREVIOUS ATTEMPT FAILED ===
The following errors occurred. Fix them:

error[E0599]: the method `clone` exists for struct `MutexGuard`...

=== YOUR PREVIOUS CODE ===
{previous code}

Fix the errors above and return the corrected complete code.
```

### 2. RollbackManager

Empêche les régressions en trackant la qualité de chaque tentative.

**Score de qualité:**
```rust
if !compiles {
    score = -(compile_errors * 100)  // Négatif si compile pas
} else {
    score = (tests_passed * 10) - (tests_failed * 5)  // Positif si compile
}
```

**Déclencheurs de rollback:**
- Code compilait → compile plus
- 2+ dégradations consécutives
- Régression significative des tests

**Actions:**
```
Attempt 1: score -300 (3 compile errors)
Attempt 2: score -200 (2 compile errors) ← Best
Attempt 3: score -300 (regression)
Attempt 4: score 0 (compiles!)           ← New Best
Attempt 5: score -200 (regression)       → ROLLBACK to Attempt 4
```

### 3. BusLinter (Fast Pre-validation)

Validation syntaxique rapide avant la compilation coûteuse:

| Check | Temps |
|-------|-------|
| Balanced braces `{}` | ~1ms |
| Balanced brackets `[]` | ~1ms |
| Balanced parentheses `()` | ~1ms |
| Basic syntax errors | ~1ms |
| **Total** | **~3ms** |

vs Compilation complète: **~25 secondes**

### 4. Targeted Fix Prompts

Quand on a un code qui compile mais avec des tests qui échouent:

```rust
=== BEST WORKING CODE (COMPILES, ALMOST PASSES) ===
{code}

=== SINGLE FAILING TEST ===
Only test_scheduler_cycle_detection is failing.

=== YOUR TASK ===
Fix ONLY the logic that makes this specific test fail.
DO NOT change other parts of the code that work.
```

### 5. Error Fingerprinting

Classification automatique des erreurs pour router vers le bon modèle:

```rust
use agentic_core::distributed::bus::{ErrorFingerprinter, ModelTier, ErrorCategory};

let fingerprinter = ErrorFingerprinter::new();
let analysis = fingerprinter.analyze(&errors);

match analysis.recommended_tier {
    ModelTier::Fast => {
        // Simple errors: imports, syntax, unknown types
        // E0432, E0433, E0412, syntax errors
        // → gemini-2.0-flash (cheap, fast)
    }
    ModelTier::Smart => {
        // Complex errors: ownership, borrow checker, type system
        // E0382, E0277, E0308, E0597
        // → gemini-3-flash-preview (expensive, smart)
    }
    ModelTier::Expert => {
        // Very complex: lifetimes, advanced generics
        // E0106, E0495, E0207
        // → Reserved for future use
    }
}
```

**Catégories d'erreurs:**

| Catégorie | Codes | Tier |
|-----------|-------|------|
| `Lookup` | E0432, E0433, E0412, E0425 | Fast |
| `Syntax` | unclosed delimiter, expected | Fast |
| `Ownership` | E0382, E0505, E0502, E0507 | Smart |
| `TypeSystem` | E0277, E0308, E0599 | Smart |
| `Async` | E0728, future, async | Smart |
| `Lifetime` | E0106, E0495, E0207 | Expert |
| `TestFailure` | assertion failed, panicked | Fast |
| `Unknown` | autres | Fast |

**Indicateurs visuels:**
- ⚡ Fast model
- 🧠 Smart model
- 🎓 Expert model

### 6. Clippy Integration

Analyse statique avec Clippy pour détecter les anti-patterns:

```rust
// Dans SandboxPipelineValidator, Clippy est exécuté après la compilation
// et avant les tests

// Erreurs Clippy détectées:
// - permit.forget() → memory leak potentiel
// - .clone() sur Arc au lieu de Arc::clone()
// - Unused variables
// - Inefficient patterns
```

**Avantages:**
- Détecte des bugs subtils que la compilation ne voit pas
- Améliore la qualité du code généré
- Feedback spécifique pour le LLM

### 7. MCP Doc Validator

Enrichissement des erreurs avec de la documentation via Context7:

```rust
use agentic_core::distributed::{MCPDocValidator, MCPDocConfig};

// Optionnel: activé si CONTEXT7_API_KEY est défini
let mcp_config = MCPDocConfig::context7(&api_key);
let mcp_validator = MCPDocValidator::new(mcp_config);

// Le pipeline devient:
let pipeline = ValidatorPipeline::new()
    .add(sandbox_validator)  // Compile + Clippy + Tests
    .add(mcp_validator);     // Enrichit avec docs si erreur
```

**Fonctionnement:**
1. Si erreurs de compilation → recherche docs pertinentes
2. Extrait keywords des erreurs (ex: "clone", "Arc", "Semaphore")
3. Query Context7 pour documentation up-to-date
4. Ajoute les docs au feedback pour le LLM

**Exemple de sortie:**
```
Fetching docs for topic: rust clone
  → Found 5 relevant documentation snippets
  → Adding to error context for LLM
```

## Configuration

### Variables d'environnement

| Variable | Description | Défaut |
|----------|-------------|--------|
| `NATS_URL` | URL du serveur NATS | `nats://localhost:4222` |
| `GEMINI_API_KEY` | Clé API Gemini (obligatoire) | - |
| `FAST_MODEL` | Modèle pour erreurs simples | `gemini-2.0-flash` |
| `SMART_MODEL` | Modèle pour erreurs complexes | `gemini-2.0-flash` |
| `CONTEXT7_API_KEY` | Active MCP Doc Validator | - (optionnel) |

### Constantes

```rust
const MAX_STAGE_RETRIES: u32 = 7;  // Plus de retries avec escalation de modèle
```

### Timeouts

```rust
// Sandbox timeout pour compilation + tests
let sandbox = ProcessSandbox::new().with_timeout(120_000);  // 2 minutes
```

### Configuration des modèles

```rust
// Fast model: cheap, rapide, bon pour erreurs simples
let fast_model = std::env::var("FAST_MODEL")
    .unwrap_or_else(|_| "gemini-2.0-flash".to_string());

// Smart model: coûteux, intelligent, bon pour erreurs complexes
let smart_model = std::env::var("SMART_MODEL")
    .unwrap_or_else(|_| "gemini-2.0-flash".to_string());
```

### NATS

```rust
let nats_url = std::env::var("NATS_URL")
    .unwrap_or_else(|_| "nats://localhost:4222".to_string());
```

## Résultats typiques

### Exécution réussie (avec toutes les fonctionnalités)

```
============================================================
  PROGRESSIVE LOCKING PIPELINE
  Types → Stubs → Logic (lock each stage)
  + Error Fingerprinting (Fast → Smart escalation)
============================================================

✓ NATS connected
✓ Fast model: gemini-2.0-flash
✓ Smart model: gemini-3-flash-preview
✓ MCP Doc Validator enabled (Context7)

[Stage 1/3] TYPES
  Attempt 1/7 ⚡
    ✓ Lint OK
  ✓ Types LOCKED (892 chars)

[Stage 2/3] STUBS
  Attempt 1/7 ⚡
    ✓ Lint OK
  ✓ Stubs LOCKED (2156 chars)

[Stage 3/3] LOGIC
  Attempt 1/7 ⚡
    Validating...
    ✗ 5 errors (score: -500) [TypeSystem→Smart]
      - error[E0599]: no method named `clone` found...
  Attempt 2/7 🧠
    Validating...
    Fetching docs for topic: rust clone
    ✗ 3 errors (score: -300, best: -300) [Ownership→Smart]
  Attempt 3/7 🧠
    Validating...
    ✗ 2 errors (score: -200, best: -200) [TypeSystem→Smart]
  Attempt 4/7 🧠
    Validating...
    ✗ 1 errors (score: 10, best: 10) [TestFailure→Fast]
  Attempt 5/7 ⚡
    Validating...
    ↩ ROLLBACK: Degraded by 2 tests
  Attempt 6/7 ⚡
    Validating...
    ✓ Validation passed

============================================================
SUCCESS in 290643ms
============================================================
```

### Légende des indicateurs

| Icône | Signification |
|-------|---------------|
| ⚡ | Fast model (erreurs simples) |
| 🧠 | Smart model (erreurs complexes) |
| 🎓 | Expert model (lifetimes) |
| ↩ | Rollback au meilleur code |

### Métriques

| Métrique | Avant (single model) | Après (fingerprinting) |
|----------|---------------------|------------------------|
| Temps total | 3-6 minutes | 4-8 minutes |
| Tentatives Types | 1-2 | 1 |
| Tentatives Stubs | 1-3 | 1-2 |
| Tentatives Logic | 3-5 | 4-7 |
| Taux de succès | ~80% | ~85% |
| Qualité du code | 5-6/10 | 7-8/10 |
| Coût tokens | $$ | $ (fast model pour 60%+ des retries) |

## Limitations actuelles

1. **Qualité du code généré:** ~7-8/10
   - Le code compile et passe les tests
   - Clippy détecte les anti-patterns
   - Mais peut encore avoir des bugs subtils (algorithmes complexes)

2. **Règles hardcodées dans le prompt:**
   - "NO async recursion"
   - "Use Arc<Semaphore>"
   - Amélioration possible: extraction automatique des règles du task

3. **MCP Doc limité:**
   - Dépend de Context7 pour la documentation
   - Latence supplémentaire (~5s par query)

4. **Expert model non utilisé:**
   - Tier Expert (lifetimes) détecté mais route vers Smart
   - Amélioration possible: intégration d'un modèle très puissant (Claude Opus)

## Fonctionnalités implémentées

### Phase 2.2 (complétée)
- [x] **Error Fingerprinting**: Classification automatique des erreurs
- [x] **Model Hierarchy**: Fast model → Smart model escalation
- [x] **Clippy Integration**: Détection des anti-patterns
- [x] **MCP Doc Validator**: Enrichissement avec documentation Context7

### Améliorations futures

### Court terme
- [ ] Expert tier avec Claude Opus pour lifetimes
- [ ] Cache des docs MCP pour éviter re-fetch
- [ ] Métriques de coût (tokens Fast vs Smart)

### Moyen terme
- [ ] Stage de Review avec critique du code par gros LLM
- [ ] Patterns library (exemples de code correct Rust)
- [ ] Métriques de qualité du code (complexité cyclomatique, coverage)

### Long terme
- [ ] Multi-agent avec spécialisation (Types expert, Logic expert)
- [ ] Apprentissage des erreurs communes par projet
- [ ] Intégration IDE pour feedback en temps réel
- [ ] Fine-tuning du fingerprinter sur historique d'erreurs

## Usage

```bash
# Démarrer NATS (requis)
docker run -p 4222:4222 nats:latest

# Configuration minimale
export GEMINI_API_KEY="your-key"

# Lancer le pipeline (fast model uniquement)
cargo run --example multi_executor

# Configuration recommandée (avec toutes les fonctionnalités)
export GEMINI_API_KEY="your-key"
export FAST_MODEL="gemini-2.0-flash"
export SMART_MODEL="gemini-3-flash-preview"  # ou gemini-2.5-flash-preview-05-20
export CONTEXT7_API_KEY="your-context7-key"  # Optionnel

cargo run --example multi_executor
```

## Code source

| Fichier | Description |
|---------|-------------|
| `examples/multi_executor.rs` | Pipeline principal |
| `crates/agentic-core/src/distributed/bus/linter.rs` | BusLinter (fast pre-validation) |
| `crates/agentic-core/src/distributed/bus/rollback.rs` | RollbackManager |
| `crates/agentic-core/src/distributed/bus/error_fingerprint.rs` | ErrorFingerprinter |
| `crates/agentic-core/src/distributed/sandbox_validator.rs` | SandboxValidator + Clippy |
| `crates/agentic-core/src/distributed/mcp_doc_validator.rs` | MCPDocValidator |
| `crates/agentic-core/src/distributed/bus/state_store.rs` | StateStore |

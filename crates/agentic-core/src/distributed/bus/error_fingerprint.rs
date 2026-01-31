//! Error Fingerprinting - Classify errors and route to appropriate model tier.
//!
//! Simple errors → Fast model (cheap)
//! Complex errors → Smart model (expensive but better)
//!
//! # Error Classification
//!
//! | Error Code | Nature | Model Tier |
//! |------------|--------|------------|
//! | E0432 | Import missing | Fast |
//! | E0433 | Module not found | Fast |
//! | E0425 | Variable not found | Fast |
//! | E0382 | Borrow checker (move) | Smart |
//! | E0502 | Borrow checker (mut) | Smart |
//! | E0277 | Trait not implemented | Smart |
//! | E0308 | Type mismatch | Smart |
//! | E0733 | Async recursion | Smart |

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Model tier for error resolution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelTier {
    /// Fast, cheap model for simple errors
    Fast,
    /// Smart, expensive model for complex errors
    Smart,
    /// Expert model for very complex errors (optional)
    Expert,
}

impl ModelTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelTier::Fast => "fast",
            ModelTier::Smart => "smart",
            ModelTier::Expert => "expert",
        }
    }
}

/// Error category based on complexity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCategory {
    /// Simple lookup errors (imports, variables)
    Lookup,
    /// Syntax errors (braces, semicolons)
    Syntax,
    /// Ownership/borrowing errors
    Ownership,
    /// Type system errors
    TypeSystem,
    /// Async-related errors
    Async,
    /// Lifetime errors
    Lifetime,
    /// Test failures (logic errors)
    TestFailure,
    /// Timeout (deadlock, infinite loop)
    Timeout,
    /// Unknown/other
    Unknown,
}

/// Fingerprint of an error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorFingerprint {
    /// Rust error code (e.g., "E0382")
    pub code: Option<String>,
    /// Category of the error
    pub category: ErrorCategory,
    /// Recommended model tier
    pub recommended_tier: ModelTier,
    /// Short description
    pub description: String,
    /// Whether this error typically needs context about types
    pub needs_type_context: bool,
    /// Whether this error typically needs full code context
    pub needs_full_context: bool,
}

/// Error Fingerprinter - Classifies errors and recommends model tier
pub struct ErrorFingerprinter {
    /// Known error patterns
    patterns: HashMap<String, ErrorFingerprint>,
}

impl Default for ErrorFingerprinter {
    fn default() -> Self {
        Self::new()
    }
}

impl ErrorFingerprinter {
    pub fn new() -> Self {
        let mut patterns = HashMap::new();

        // === FAST TIER (Simple lookup) ===

        patterns.insert(
            "E0432".to_string(),
            ErrorFingerprint {
                code: Some("E0432".to_string()),
                category: ErrorCategory::Lookup,
                recommended_tier: ModelTier::Fast,
                description: "Unresolved import".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            },
        );

        patterns.insert(
            "E0433".to_string(),
            ErrorFingerprint {
                code: Some("E0433".to_string()),
                category: ErrorCategory::Lookup,
                recommended_tier: ModelTier::Fast,
                description: "Failed to resolve module".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            },
        );

        patterns.insert(
            "E0425".to_string(),
            ErrorFingerprint {
                code: Some("E0425".to_string()),
                category: ErrorCategory::Lookup,
                recommended_tier: ModelTier::Fast,
                description: "Cannot find value in scope".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            },
        );

        patterns.insert(
            "E0412".to_string(),
            ErrorFingerprint {
                code: Some("E0412".to_string()),
                category: ErrorCategory::Lookup,
                recommended_tier: ModelTier::Fast,
                description: "Cannot find type in scope".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            },
        );

        // E0599 can be simple (method not found) or complex (trait bounds not satisfied)
        // We classify it as Smart because "trait bounds not satisfied" is common
        patterns.insert(
            "E0599".to_string(),
            ErrorFingerprint {
                code: Some("E0599".to_string()),
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Smart,
                description: "Method not found or trait bounds not satisfied".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            },
        );

        // === SMART TIER (Ownership/Borrowing) ===

        patterns.insert(
            "E0382".to_string(),
            ErrorFingerprint {
                code: Some("E0382".to_string()),
                category: ErrorCategory::Ownership,
                recommended_tier: ModelTier::Smart,
                description: "Use of moved value".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            },
        );

        patterns.insert(
            "E0502".to_string(),
            ErrorFingerprint {
                code: Some("E0502".to_string()),
                category: ErrorCategory::Ownership,
                recommended_tier: ModelTier::Smart,
                description: "Cannot borrow as mutable (already borrowed)".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            },
        );

        patterns.insert(
            "E0503".to_string(),
            ErrorFingerprint {
                code: Some("E0503".to_string()),
                category: ErrorCategory::Ownership,
                recommended_tier: ModelTier::Smart,
                description: "Cannot use value because it was mutably borrowed".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            },
        );

        patterns.insert(
            "E0505".to_string(),
            ErrorFingerprint {
                code: Some("E0505".to_string()),
                category: ErrorCategory::Ownership,
                recommended_tier: ModelTier::Smart,
                description: "Cannot move out of borrowed content".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            },
        );

        patterns.insert(
            "E0507".to_string(),
            ErrorFingerprint {
                code: Some("E0507".to_string()),
                category: ErrorCategory::Ownership,
                recommended_tier: ModelTier::Smart,
                description: "Cannot move out of borrowed content".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            },
        );

        // === SMART TIER (Type System) ===

        patterns.insert(
            "E0277".to_string(),
            ErrorFingerprint {
                code: Some("E0277".to_string()),
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Smart,
                description: "Trait bound not satisfied".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            },
        );

        patterns.insert(
            "E0308".to_string(),
            ErrorFingerprint {
                code: Some("E0308".to_string()),
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Smart,
                description: "Mismatched types".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            },
        );

        patterns.insert(
            "E0282".to_string(),
            ErrorFingerprint {
                code: Some("E0282".to_string()),
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Fast,
                description: "Type annotations needed".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            },
        );

        // === SMART TIER (Async) ===

        patterns.insert(
            "E0733".to_string(),
            ErrorFingerprint {
                code: Some("E0733".to_string()),
                category: ErrorCategory::Async,
                recommended_tier: ModelTier::Smart,
                description: "Async recursion detected".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            },
        );

        patterns.insert(
            "E0728".to_string(),
            ErrorFingerprint {
                code: Some("E0728".to_string()),
                category: ErrorCategory::Async,
                recommended_tier: ModelTier::Smart,
                description: "await used outside of async context".to_string(),
                needs_type_context: false,
                needs_full_context: true,
            },
        );

        // === EXPERT TIER (Lifetimes) ===

        patterns.insert(
            "E0106".to_string(),
            ErrorFingerprint {
                code: Some("E0106".to_string()),
                category: ErrorCategory::Lifetime,
                recommended_tier: ModelTier::Expert,
                description: "Missing lifetime specifier".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            },
        );

        patterns.insert(
            "E0495".to_string(),
            ErrorFingerprint {
                code: Some("E0495".to_string()),
                category: ErrorCategory::Lifetime,
                recommended_tier: ModelTier::Expert,
                description: "Cannot infer appropriate lifetime".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            },
        );

        patterns.insert(
            "E0521".to_string(),
            ErrorFingerprint {
                code: Some("E0521".to_string()),
                category: ErrorCategory::Lifetime,
                recommended_tier: ModelTier::Expert,
                description: "Borrowed data escapes outside of method".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            },
        );

        // === TYPESCRIPT ERRORS ===

        // TS1005: ',' expected (and similar syntax errors)
        patterns.insert(
            "TS1005".to_string(),
            ErrorFingerprint {
                code: Some("TS1005".to_string()),
                category: ErrorCategory::Syntax,
                recommended_tier: ModelTier::Fast,
                description: "TypeScript syntax error - missing delimiter".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            },
        );

        // TS1109: Expression expected
        patterns.insert(
            "TS1109".to_string(),
            ErrorFingerprint {
                code: Some("TS1109".to_string()),
                category: ErrorCategory::Syntax,
                recommended_tier: ModelTier::Fast,
                description: "TypeScript syntax error - expression expected".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            },
        );

        // TS1128: Declaration or statement expected
        patterns.insert(
            "TS1128".to_string(),
            ErrorFingerprint {
                code: Some("TS1128".to_string()),
                category: ErrorCategory::Syntax,
                recommended_tier: ModelTier::Fast,
                description: "TypeScript syntax error - declaration expected".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            },
        );

        // TS2304: Cannot find name 'xxx'
        patterns.insert(
            "TS2304".to_string(),
            ErrorFingerprint {
                code: Some("TS2304".to_string()),
                category: ErrorCategory::Lookup,
                recommended_tier: ModelTier::Fast,
                description: "TypeScript cannot find name".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            },
        );

        // TS2339: Property 'xxx' does not exist on type
        patterns.insert(
            "TS2339".to_string(),
            ErrorFingerprint {
                code: Some("TS2339".to_string()),
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Smart,
                description: "TypeScript property does not exist".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            },
        );

        // TS2345: Argument of type 'X' is not assignable to parameter of type 'Y'
        patterns.insert(
            "TS2345".to_string(),
            ErrorFingerprint {
                code: Some("TS2345".to_string()),
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Smart,
                description: "TypeScript type mismatch in argument".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            },
        );

        // TS2322: Type 'X' is not assignable to type 'Y'
        patterns.insert(
            "TS2322".to_string(),
            ErrorFingerprint {
                code: Some("TS2322".to_string()),
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Smart,
                description: "TypeScript type not assignable".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            },
        );

        // TS7006: Parameter 'x' implicitly has an 'any' type
        patterns.insert(
            "TS7006".to_string(),
            ErrorFingerprint {
                code: Some("TS7006".to_string()),
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Fast,
                description: "TypeScript implicit any type".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            },
        );

        // TS2307: Cannot find module 'xxx'
        patterns.insert(
            "TS2307".to_string(),
            ErrorFingerprint {
                code: Some("TS2307".to_string()),
                category: ErrorCategory::Lookup,
                recommended_tier: ModelTier::Fast,
                description: "TypeScript cannot find module".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            },
        );

        // TS2551: Property 'x' does not exist. Did you mean 'y'?
        patterns.insert(
            "TS2551".to_string(),
            ErrorFingerprint {
                code: Some("TS2551".to_string()),
                category: ErrorCategory::Lookup,
                recommended_tier: ModelTier::Fast,
                description: "TypeScript property typo".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            },
        );

        // TS2571: Object is of type 'unknown'
        patterns.insert(
            "TS2571".to_string(),
            ErrorFingerprint {
                code: Some("TS2571".to_string()),
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Smart,
                description: "TypeScript unknown type".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            },
        );

        // TS2532: Object is possibly 'undefined'
        patterns.insert(
            "TS2532".to_string(),
            ErrorFingerprint {
                code: Some("TS2532".to_string()),
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Fast,
                description: "TypeScript possibly undefined".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            },
        );

        // TS1003: Identifier expected
        patterns.insert(
            "TS1003".to_string(),
            ErrorFingerprint {
                code: Some("TS1003".to_string()),
                category: ErrorCategory::Syntax,
                recommended_tier: ModelTier::Fast,
                description: "TypeScript identifier expected".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            },
        );

        // TS18046: 'x' is of type 'unknown'
        patterns.insert(
            "TS18046".to_string(),
            ErrorFingerprint {
                code: Some("TS18046".to_string()),
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Smart,
                description: "TypeScript value is unknown type".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            },
        );

        Self { patterns }
    }

    /// Fingerprint a single error string
    pub fn fingerprint(&self, error: &str) -> ErrorFingerprint {
        // Try to extract error code (e.g., "error[E0382]")
        if let Some(code) = Self::extract_error_code(error) {
            if let Some(fp) = self.patterns.get(&code) {
                return fp.clone();
            }
        }

        // Check for timeout errors (deadlock, infinite loop)
        if error.contains("timed out")
            || error.contains("Execution timed out")
            || error.contains("TIMEOUT")
        {
            return ErrorFingerprint {
                code: None,
                category: ErrorCategory::Timeout,
                recommended_tier: ModelTier::Smart,
                description: "Execution timeout (probable deadlock or infinite loop)".to_string(),
                needs_type_context: false,
                needs_full_context: true,
            };
        }

        // Check for test failures
        if error.contains("FAILED") || error.contains("panicked at") {
            return ErrorFingerprint {
                code: None,
                category: ErrorCategory::TestFailure,
                recommended_tier: ModelTier::Smart,
                description: "Test failure".to_string(),
                needs_type_context: false,
                needs_full_context: true,
            };
        }

        // Check for async/Send errors (SMART tier - complex)
        let error_lower = error.to_lowercase();
        if error_lower.contains("cannot be sent between threads")
            || error_lower.contains("future cannot be sent")
            || error_lower.contains("is not send")
            || error_lower.contains("doesn't implement `send`")
            || error_lower.contains("within `impl future")
        {
            return ErrorFingerprint {
                code: None,
                category: ErrorCategory::Async,
                recommended_tier: ModelTier::Smart,
                description: "Async Send bound error".to_string(),
                needs_type_context: true,
                needs_full_context: true,
            };
        }

        // Check for TypeScript/JavaScript syntax errors (case insensitive)
        if error_lower.contains("unclosed delimiter")
            || error_lower.contains("unexpected token")
            || error_lower.contains("lint error")
            || error_lower.contains("syntaxerror")
            || error_lower.contains("expected")  // TypeScript: ',' expected, ';' expected
            || error_lower.contains("declaration or statement expected")
            || error_lower.contains("expression expected")
        {
            return ErrorFingerprint {
                code: None,
                category: ErrorCategory::Syntax,
                recommended_tier: ModelTier::Fast,
                description: "Syntax error".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            };
        }

        // Check for TypeScript type errors
        if error_lower.contains("is not assignable to")
            || error_lower.contains("does not exist on type")
            || error_lower.contains("property") && error_lower.contains("does not exist")
            || error_lower.contains("argument of type")
            || error_lower.contains("cannot find name")
        {
            return ErrorFingerprint {
                code: None,
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Smart,
                description: "TypeScript type error".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            };
        }

        // Check for Jest/TypeScript test failures
        if error_lower.contains("expect(")
            || error_lower.contains("tobecalledwith")
            || error_lower.contains("tohavebeencalled")
            || error_lower.contains("tobe(")
            || error_lower.contains("toequal")
            || error_lower.contains("jest")
        {
            return ErrorFingerprint {
                code: None,
                category: ErrorCategory::TestFailure,
                recommended_tier: ModelTier::Smart,
                description: "Jest test failure".to_string(),
                needs_type_context: false,
                needs_full_context: true,
            };
        }

        // Check for Go errors
        if error_lower.contains("undefined:")
            || error_lower.contains("cannot use")
            || error_lower.contains("too many arguments")
            || error_lower.contains("too few arguments")
            || error_lower.contains("not enough arguments")
        {
            return ErrorFingerprint {
                code: None,
                category: ErrorCategory::Lookup,
                recommended_tier: ModelTier::Fast,
                description: "Go compilation error".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            };
        }

        // Check for Python errors
        if error_lower.contains("nameerror")
            || error_lower.contains("importerror")
            || error_lower.contains("modulenotfounderror")
        {
            return ErrorFingerprint {
                code: None,
                category: ErrorCategory::Lookup,
                recommended_tier: ModelTier::Fast,
                description: "Python import/name error".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            };
        }

        if error_lower.contains("typeerror") || error_lower.contains("attributeerror") {
            return ErrorFingerprint {
                code: None,
                category: ErrorCategory::TypeSystem,
                recommended_tier: ModelTier::Smart,
                description: "Python type/attribute error".to_string(),
                needs_type_context: true,
                needs_full_context: false,
            };
        }

        if error_lower.contains("indentationerror") || error_lower.contains("invalid syntax") {
            return ErrorFingerprint {
                code: None,
                category: ErrorCategory::Syntax,
                recommended_tier: ModelTier::Fast,
                description: "Python syntax error".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            };
        }

        // Meta errors (not actionable, should be ignored or treated as simple)
        if error_lower.contains("could not compile")
            || error_lower.contains("aborting due to")
            || error_lower.contains("previous error")
        {
            return ErrorFingerprint {
                code: None,
                category: ErrorCategory::Syntax, // Treat as syntax (follows the real error)
                recommended_tier: ModelTier::Fast,
                description: "Compilation meta-error".to_string(),
                needs_type_context: false,
                needs_full_context: false,
            };
        }

        // Unknown error - default to Fast to avoid unnecessary escalation
        // Only escalate when we have a specific complex error code
        ErrorFingerprint {
            code: None,
            category: ErrorCategory::Unknown,
            recommended_tier: ModelTier::Fast, // Changed from Smart
            description: "Unknown error".to_string(),
            needs_type_context: false,
            needs_full_context: false,
        }
    }

    /// Fingerprint multiple errors and determine the recommended tier
    pub fn analyze(&self, errors: &[String]) -> AnalysisResult {
        let fingerprints: Vec<_> = errors.iter().map(|e| self.fingerprint(e)).collect();

        // Determine the highest tier needed (from real errors, not meta-errors)
        let recommended_tier = fingerprints
            .iter()
            .filter(|fp| !Self::is_meta_error(fp))
            .map(|fp| fp.recommended_tier)
            .max_by_key(|t| match t {
                ModelTier::Fast => 0,
                ModelTier::Smart => 1,
                ModelTier::Expert => 2,
            })
            .unwrap_or(ModelTier::Fast);

        // Count by category (excluding meta-errors like "could not compile")
        let mut category_counts: HashMap<ErrorCategory, usize> = HashMap::new();
        for fp in &fingerprints {
            // Skip meta-errors in category count - they're not actionable
            if Self::is_meta_error(fp) {
                continue;
            }
            *category_counts.entry(fp.category).or_insert(0) += 1;
        }

        // Determine dominant category (from real errors only)
        let dominant_category = category_counts
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(cat, _)| *cat)
            .unwrap_or(ErrorCategory::Unknown);

        // Check if we need full context
        let needs_full_context = fingerprints.iter().any(|fp| fp.needs_full_context);
        let needs_type_context = fingerprints.iter().any(|fp| fp.needs_type_context);

        AnalysisResult {
            fingerprints,
            recommended_tier,
            dominant_category,
            category_counts,
            needs_full_context,
            needs_type_context,
        }
    }

    /// Check if a fingerprint is a meta-error (not actionable, just noise)
    /// Meta-errors are compilation summary messages like "could not compile" or "aborting due to"
    fn is_meta_error(fp: &ErrorFingerprint) -> bool {
        fp.description == "Compilation meta-error"
    }

    /// Extract error code from error message (e.g., "E0382" from "error[E0382]" or "TS1005" from "error TS1005:")
    fn extract_error_code(error: &str) -> Option<String> {
        // TypeScript: look for "error TS1005:" or "TS1005:" pattern
        if let Some(pos) = error.find("TS") {
            let rest = &error[pos..];
            let code: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
            if code.len() >= 5
                && code.starts_with("TS")
                && code[2..].chars().all(|c| c.is_ascii_digit())
            {
                return Some(code);
            }
        }

        // Rust: look for pattern "error[E0XXX]"
        if let Some(start) = error.find("error[E") {
            let rest = &error[start + 7..];
            if let Some(end) = rest.find(']') {
                let code = &rest[..end];
                if code.len() == 4 && code.chars().all(|c| c.is_ascii_digit()) {
                    return Some(format!("E{}", code));
                }
            }
        }

        // Rust: fallback - look for E followed by 4 digits
        if let Some(pos) = error.find("E0") {
            let code: String = error[pos..].chars().take(5).collect();
            if code.len() == 5 && code[1..].chars().all(|c| c.is_ascii_digit()) {
                return Some(code);
            }
        }

        None
    }

    /// Generate a specialized prompt hint based on error analysis
    pub fn generate_hint(&self, analysis: &AnalysisResult) -> String {
        let mut hints = vec![];

        match analysis.dominant_category {
            ErrorCategory::Ownership => {
                hints.push("Focus on ownership and borrowing. Consider:");
                hints.push("- Use Arc::clone(&x) instead of x.clone() for Arc");
                hints.push("- Use references (&T) instead of moving values");
                hints.push("- Clone data before moving into closures");
            }
            ErrorCategory::TypeSystem => {
                hints.push("Focus on type constraints. Consider:");
                hints.push("- Add #[derive(Clone)] if Clone is needed");
                hints.push("- Check trait bounds match the usage");
                hints.push("- Use explicit type annotations");
            }
            ErrorCategory::Async => {
                hints.push("Focus on async patterns. Consider:");
                hints.push("- NO async recursion - use loop + Vec as stack");
                hints.push("- Use Arc<Semaphore> for concurrency control");
                hints.push("- Ensure futures are Send + 'static for spawn");
            }
            ErrorCategory::Lifetime => {
                hints.push("Focus on lifetimes. Consider:");
                hints.push("- Add explicit lifetime parameters");
                hints.push("- Use 'static for data that lives forever");
                hints.push("- Consider owned types instead of references");
            }
            ErrorCategory::Lookup => {
                hints.push("Focus on imports and scope. Consider:");
                hints.push("- Add missing use statements");
                hints.push("- Check module paths are correct");
                hints.push("- Verify struct/method names");
            }
            ErrorCategory::TestFailure => {
                hints.push("Focus on test logic. Consider:");
                hints.push("- Check assertions match expected behavior");
                hints.push("- Verify edge cases are handled");
                hints.push("- Debug with println! if needed");
            }
            ErrorCategory::Timeout => {
                hints.push("TIMEOUT detected - likely deadlock or infinite loop. Consider:");
                hints.push("- Check threading: avoid Lock while holding another Lock");
                hints.push("- Check wait loops: add timeout to wait_and_acquire");
                hints.push("- Check recursion: ensure base case is reachable");
                hints.push("- Use threading.RLock instead of Lock if needed");
                hints.push("- Add timeout parameter to blocking operations");
            }
            ErrorCategory::Syntax => {
                hints.push("Focus on syntax. Consider:");
                hints.push("- Check for missing/extra commas, semicolons, braces");
                hints.push("- Verify function/class declarations are complete");
                hints.push("- Check for unclosed strings or template literals");
                hints.push(
                    "- IMPORTANT: Template strings MUST use backticks: `text ${var}` NOT quotes",
                );
                hints.push("- Ensure proper indentation (Python)");
            }
            ErrorCategory::Unknown => {
                hints.push("Unknown error category. Consider:");
                hints.push("- Read the full error message carefully");
                hints.push("- Check the line number mentioned in the error");
                hints.push("- Verify all imports and dependencies exist");
            }
        }

        hints.join("\n")
    }
}

/// Result of analyzing multiple errors
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// Individual fingerprints
    pub fingerprints: Vec<ErrorFingerprint>,
    /// Recommended model tier (highest needed)
    pub recommended_tier: ModelTier,
    /// Dominant error category
    pub dominant_category: ErrorCategory,
    /// Count by category
    pub category_counts: HashMap<ErrorCategory, usize>,
    /// Whether full code context is needed
    pub needs_full_context: bool,
    /// Whether type context is needed
    pub needs_type_context: bool,
}

impl AnalysisResult {
    /// Get a summary string
    pub fn summary(&self) -> String {
        let tier = self.recommended_tier.as_str();
        let category = format!("{:?}", self.dominant_category);
        format!(
            "Tier: {}, Category: {}, Errors: {}",
            tier,
            category,
            self.fingerprints.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_error_code() {
        let fp = ErrorFingerprinter::new();

        assert_eq!(
            ErrorFingerprinter::extract_error_code("error[E0382]: use of moved value"),
            Some("E0382".to_string())
        );

        assert_eq!(
            ErrorFingerprinter::extract_error_code("E0277: trait bound not satisfied"),
            Some("E0277".to_string())
        );
    }

    #[test]
    fn test_fingerprint_ownership_error() {
        let fp = ErrorFingerprinter::new();
        let result = fp.fingerprint("error[E0382]: use of moved value: `x`");

        assert_eq!(result.category, ErrorCategory::Ownership);
        assert_eq!(result.recommended_tier, ModelTier::Smart);
    }

    #[test]
    fn test_fingerprint_import_error() {
        let fp = ErrorFingerprinter::new();
        let result = fp.fingerprint("error[E0432]: unresolved import `foo`");

        assert_eq!(result.category, ErrorCategory::Lookup);
        assert_eq!(result.recommended_tier, ModelTier::Fast);
    }

    #[test]
    fn test_analyze_multiple_errors() {
        let fp = ErrorFingerprinter::new();
        let errors = vec![
            "error[E0432]: unresolved import".to_string(),
            "error[E0382]: use of moved value".to_string(),
            "error[E0382]: use of moved value".to_string(),
        ];

        let result = fp.analyze(&errors);

        // Should recommend Smart tier due to E0382
        assert_eq!(result.recommended_tier, ModelTier::Smart);
        // Dominant category should be Ownership (2 vs 1)
        assert_eq!(result.dominant_category, ErrorCategory::Ownership);
    }

    #[test]
    fn test_generate_hint() {
        let fp = ErrorFingerprinter::new();
        let errors = vec!["error[E0382]: use of moved value".to_string()];
        let analysis = fp.analyze(&errors);

        let hint = fp.generate_hint(&analysis);
        assert!(hint.contains("ownership"));
    }
}

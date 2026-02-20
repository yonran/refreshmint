use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

use crate::account_journal::{self, AccountEntry, EntryPosting, EntryStatus, SimpleAmount};

/// A proposed transaction from extraction (matches the JS API schema).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedTransaction {
    pub tdate: String,
    #[serde(default = "default_status_string")]
    pub tstatus: String,
    #[serde(default)]
    pub tdescription: String,
    #[serde(default)]
    pub tcomment: String,
    #[serde(default)]
    pub ttags: Vec<(String, String)>,
    #[serde(default)]
    pub tpostings: Option<Vec<ExtractedPosting>>,
}

fn default_status_string() -> String {
    "Unmarked".to_string()
}

/// A posting from extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedPosting {
    pub paccount: String,
    #[serde(default)]
    pub pamount: Option<Vec<ExtractedAmount>>,
}

/// An amount from extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedAmount {
    #[serde(default)]
    pub acommodity: String,
    pub aquantity: String,
}

impl ExtractedTransaction {
    /// Get the evidence tags from ttags.
    pub fn evidence_refs(&self) -> Vec<String> {
        self.ttags
            .iter()
            .filter(|(k, _)| k == "evidence")
            .map(|(_, v)| v.clone())
            .collect()
    }

    /// Get the bankId tag value, if present.
    pub fn bank_id(&self) -> Option<&str> {
        self.ttags
            .iter()
            .find(|(k, _)| k == "bankId")
            .map(|(_, v)| v.as_str())
    }

    /// Parse the status string into EntryStatus.
    pub fn status(&self) -> EntryStatus {
        match self.tstatus.as_str() {
            "Cleared" | "cleared" | "*" => EntryStatus::Cleared,
            "Pending" | "pending" | "!" => EntryStatus::Pending,
            _ => EntryStatus::Unmarked,
        }
    }

    /// Convert to an AccountEntry with the given default account and unreconciled equity account.
    pub fn to_account_entry(
        &self,
        default_account: &str,
        unreconciled_equity: &str,
    ) -> AccountEntry {
        let evidence = self.evidence_refs();

        let postings = if let Some(explicit) = &self.tpostings {
            explicit
                .iter()
                .map(|p| {
                    let amount = p.pamount.as_ref().and_then(|amounts| {
                        amounts.first().map(|a| SimpleAmount {
                            commodity: a.acommodity.clone(),
                            quantity: a.aquantity.clone(),
                        })
                    });
                    EntryPosting {
                        account: p.paccount.clone(),
                        amount,
                    }
                })
                .collect()
        } else {
            // Default single-sided: find the first amount from tags or infer
            let amount = self.find_primary_amount();
            let mut postings = vec![EntryPosting {
                account: default_account.to_string(),
                amount: amount.clone(),
            }];
            // Add counterpart with negated amount
            let counterpart_amount = amount.map(|a| {
                let negated = if a.quantity.starts_with('-') {
                    a.quantity[1..].to_string()
                } else {
                    format!("-{}", a.quantity)
                };
                SimpleAmount {
                    commodity: a.commodity,
                    quantity: negated,
                }
            });
            postings.push(EntryPosting {
                account: unreconciled_equity.to_string(),
                amount: counterpart_amount,
            });
            postings
        };

        let mut entry = AccountEntry::new(
            self.tdate.clone(),
            self.status(),
            self.tdescription.clone(),
            evidence,
            postings,
        );

        // Add non-evidence, non-meta tags
        for (key, value) in &self.ttags {
            if key != "evidence" {
                entry.tags.push((key.clone(), value.clone()));
            }
        }

        if !self.tcomment.is_empty() {
            entry.comment = self.tcomment.clone();
        }

        entry
    }

    fn find_primary_amount(&self) -> Option<SimpleAmount> {
        // Look for an amount tag, or return None (postings will be inferred)
        for (key, value) in &self.ttags {
            if key == "amount" {
                let parts: Vec<&str> = value.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    return Some(SimpleAmount {
                        quantity: parts[0].to_string(),
                        commodity: parts[1].to_string(),
                    });
                }
                return Some(SimpleAmount {
                    quantity: value.clone(),
                    commodity: String::new(),
                });
            }
        }
        None
    }
}

/// Validate an extracted transaction.
pub fn validate_extracted_transaction(
    txn: &ExtractedTransaction,
    document_name: &str,
) -> Result<(), String> {
    let evidence = txn.evidence_refs();
    if evidence.is_empty() {
        return Err("extracted transaction must have at least one evidence tag".to_string());
    }

    // Verify that at least one evidence ref references the current document
    let references_doc = evidence.iter().any(|e| {
        e.starts_with(document_name)
            && e.get(document_name.len()..)
                .map(|rest| rest.starts_with(':') || rest.starts_with('#'))
                .unwrap_or(false)
    });
    if !references_doc {
        return Err(format!(
            "evidence tags must reference the input document '{document_name}', got: {}",
            evidence.join(", ")
        ));
    }

    if txn.tdate.is_empty() {
        return Err("extracted transaction must have a date".to_string());
    }

    Ok(())
}

/// Result of running extraction on a set of documents.
pub struct ExtractionResult {
    pub proposed_transactions: Vec<ExtractedTransaction>,
    pub document_names: Vec<String>,
}

/// Run extraction for a set of documents.
///
/// This orchestrates running extract.mjs or account.rules on each document,
/// collecting proposed transactions.
pub fn run_extraction(
    ledger_dir: &Path,
    account_name: &str,
    extension_name: &str,
    document_names: &[String],
) -> Result<ExtractionResult, Box<dyn std::error::Error + Send + Sync>> {
    let extension_dir = ledger_dir.join("extensions").join(extension_name);
    let manifest = crate::scrape::load_manifest(&extension_dir)?;
    let documents_dir = account_journal::account_documents_dir(ledger_dir, account_name);

    let mut all_proposed = Vec::new();

    for doc_name in document_names {
        let doc_path = documents_dir.join(doc_name);
        if !doc_path.exists() {
            return Err(format!("document not found: {}", doc_path.display()).into());
        }

        // Check for extract.mjs script
        if let Some(ref extract_script) = manifest.extract {
            let script_path = extension_dir.join(extract_script);
            if script_path.exists() {
                let proposed =
                    run_extract_script(&script_path, &doc_path, doc_name, &documents_dir)?;
                if !proposed.is_empty() {
                    all_proposed.extend(proposed);
                    continue;
                }
                eprintln!(
                    "[extract] {} produced no rows; trying rules fallback if configured",
                    script_path.display()
                );
            }
        }

        // Check for hledger CSV rules
        if let Some(ref rules_file) = manifest.rules {
            let rules_path = extension_dir.join(rules_file);
            if rules_path.exists() && doc_name.ends_with(".csv") {
                let proposed = run_rules_extraction(
                    &rules_path,
                    &doc_path,
                    doc_name,
                    manifest.id_field.as_deref(),
                )?;
                all_proposed.extend(proposed);
                continue;
            }
        }

        eprintln!("No extraction method for document: {doc_name}");
    }

    Ok(ExtractionResult {
        proposed_transactions: all_proposed,
        document_names: document_names.to_vec(),
    })
}

/// Run extract.mjs on a document using QuickJS sandbox.
///
/// For now, this is a placeholder that reads the script and runs it.
/// The full implementation would set up the QuickJS runtime with
/// `refreshmint.reportExtractedTransaction` and `refreshmint.readSessionFile`.
fn run_extract_script(
    _script_path: &Path,
    _doc_path: &Path,
    _doc_name: &str,
    _documents_dir: &Path,
) -> Result<Vec<ExtractedTransaction>, Box<dyn std::error::Error + Send + Sync>> {
    // Extraction via extract.mjs requires the QuickJS sandbox.
    // This will be fully wired up when the extraction sandbox is built.
    // For now, return empty (the rules-based extraction below is the primary path).
    eprintln!("[extract] extract.mjs execution not yet wired up for non-browser context");
    Ok(Vec::new())
}

/// Run hledger CSV rules-based extraction on a CSV document.
fn run_rules_extraction(
    rules_path: &Path,
    doc_path: &Path,
    doc_name: &str,
    id_field: Option<&str>,
) -> Result<Vec<ExtractedTransaction>, Box<dyn std::error::Error + Send + Sync>> {
    // Use hledger to convert CSV to JSON using the rules file
    let output = std::process::Command::new(crate::binpath::hledger_path())
        .arg("print")
        .arg("--output-format=json")
        .arg("-f")
        .arg(doc_path)
        .arg("--rules-file")
        .arg(rules_path)
        .env("GIT_CONFIG_GLOBAL", crate::ledger::NULL_DEVICE)
        .env("GIT_CONFIG_SYSTEM", crate::ledger::NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "hledger CSV extraction failed for {}: {}",
            doc_name,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let transactions: Vec<crate::hledger::Transaction> = serde_json::from_slice(&output.stdout)?;

    let mut extracted = Vec::new();
    for (line_num, txn) in transactions.iter().enumerate() {
        let evidence_ref = format!("{}:{}:1", doc_name, line_num + 1);
        let status = match txn.tstatus {
            crate::hledger::Status::Cleared => "Cleared".to_string(),
            crate::hledger::Status::Pending => "Pending".to_string(),
            crate::hledger::Status::Unmarked => "Unmarked".to_string(),
        };

        let mut tags = vec![("evidence".to_string(), evidence_ref)];

        // Extract bankId from the id-field column in hledger tags
        if let Some(id_field_name) = id_field {
            for (key, value) in &txn.ttags {
                if key == id_field_name && !value.is_empty() {
                    tags.push(("bankId".to_string(), value.clone()));
                    break;
                }
            }
        }

        let postings = if !txn.tpostings.is_empty() {
            Some(
                txn.tpostings
                    .iter()
                    .map(|p| {
                        let amount = if p.pamount.is_empty() {
                            None
                        } else {
                            Some(
                                p.pamount
                                    .iter()
                                    .map(|a| ExtractedAmount {
                                        acommodity: a.acommodity.clone(),
                                        aquantity: format_decimal_raw(&a.aquantity),
                                    })
                                    .collect(),
                            )
                        };
                        ExtractedPosting {
                            paccount: p.paccount.clone(),
                            pamount: amount,
                        }
                    })
                    .collect(),
            )
        } else {
            None
        };

        extracted.push(ExtractedTransaction {
            tdate: txn.tdate.clone(),
            tstatus: status,
            tdescription: txn.tdescription.clone(),
            tcomment: txn.tcomment.clone(),
            ttags: tags,
            tpostings: postings,
        });
    }

    Ok(extracted)
}

/// Format a DecimalRaw as a string quantity.
fn format_decimal_raw(raw: &crate::hledger::DecimalRaw) -> String {
    let mantissa = raw.decimal_mantissa.as_i64().unwrap_or(0);
    let scale = raw.decimal_places;
    if scale == 0 {
        return mantissa.to_string();
    }
    let negative = mantissa < 0;
    let abs = mantissa.unsigned_abs();
    let abs_str = abs.to_string();
    let scale_usize = scale as usize;
    if abs_str.len() <= scale_usize {
        let padded = format!("{:0>width$}", abs_str, width = scale_usize + 1);
        let (int_part, frac_part) = padded.split_at(padded.len() - scale_usize);
        let formatted = format!("{int_part}.{frac_part}");
        if negative {
            format!("-{formatted}")
        } else {
            formatted
        }
    } else {
        let (int_part, frac_part) = abs_str.split_at(abs_str.len() - scale_usize);
        let formatted = format!("{int_part}.{frac_part}");
        if negative {
            format!("-{formatted}")
        } else {
            formatted
        }
    }
}

/// List evidence documents for an account.
pub fn list_documents(ledger_dir: &Path, account_name: &str) -> io::Result<Vec<DocumentWithInfo>> {
    let documents_dir = account_journal::account_documents_dir(ledger_dir, account_name);
    if !documents_dir.exists() {
        return Ok(Vec::new());
    }

    let mut documents = Vec::new();
    for entry in std::fs::read_dir(&documents_dir)? {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.ends_with("-info.json") {
            continue;
        }
        if !entry.file_type()?.is_file() {
            continue;
        }

        let sidecar_path = documents_dir.join(format!("{file_name}-info.json"));
        let info = if sidecar_path.exists() {
            let content = std::fs::read_to_string(&sidecar_path)?;
            serde_json::from_str(&content).ok()
        } else {
            None
        };

        documents.push(DocumentWithInfo {
            filename: file_name,
            info,
        });
    }

    documents.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(documents)
}

/// A document file with its optional info sidecar.
#[derive(Debug, Clone, Serialize)]
pub struct DocumentWithInfo {
    pub filename: String,
    pub info: Option<crate::scrape::DocumentInfo>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn validate_requires_evidence() {
        let txn = ExtractedTransaction {
            tdate: "2024-01-01".to_string(),
            tstatus: "Cleared".to_string(),
            tdescription: "Test".to_string(),
            tcomment: String::new(),
            ttags: vec![],
            tpostings: None,
        };
        assert!(validate_extracted_transaction(&txn, "doc.csv").is_err());
    }

    #[test]
    fn validate_requires_matching_document() {
        let txn = ExtractedTransaction {
            tdate: "2024-01-01".to_string(),
            tstatus: "Cleared".to_string(),
            tdescription: "Test".to_string(),
            tcomment: String::new(),
            ttags: vec![("evidence".to_string(), "other-doc.csv:1:1".to_string())],
            tpostings: None,
        };
        assert!(validate_extracted_transaction(&txn, "doc.csv").is_err());
    }

    #[test]
    fn validate_accepts_valid_transaction() {
        let txn = ExtractedTransaction {
            tdate: "2024-01-01".to_string(),
            tstatus: "Cleared".to_string(),
            tdescription: "Test".to_string(),
            tcomment: String::new(),
            ttags: vec![("evidence".to_string(), "doc.csv:1:1".to_string())],
            tpostings: None,
        };
        assert!(validate_extracted_transaction(&txn, "doc.csv").is_ok());
    }

    #[test]
    fn to_account_entry_creates_single_sided() {
        let txn = ExtractedTransaction {
            tdate: "2024-02-15".to_string(),
            tstatus: "Cleared".to_string(),
            tdescription: "SHELL OIL".to_string(),
            tcomment: String::new(),
            ttags: vec![
                ("evidence".to_string(), "doc.csv:1:1".to_string()),
                ("bankId".to_string(), "FIT123".to_string()),
                ("amount".to_string(), "-21.32 USD".to_string()),
            ],
            tpostings: None,
        };

        let entry = txn.to_account_entry("Assets:Checking", "Equity:Unreconciled:Checking");
        assert_eq!(entry.date, "2024-02-15");
        assert_eq!(entry.status, EntryStatus::Cleared);
        assert_eq!(entry.postings.len(), 2);
        assert_eq!(entry.postings[0].account, "Assets:Checking");
        assert_eq!(entry.postings[1].account, "Equity:Unreconciled:Checking");
        assert!(entry.bank_id().is_some());
        assert_eq!(entry.bank_id().unwrap(), "FIT123");
    }

    #[test]
    fn to_account_entry_uses_explicit_postings() {
        let txn = ExtractedTransaction {
            tdate: "2024-02-15".to_string(),
            tstatus: "Cleared".to_string(),
            tdescription: "Venmo transfer".to_string(),
            tcomment: String::new(),
            ttags: vec![("evidence".to_string(), "doc.csv:1:1".to_string())],
            tpostings: Some(vec![
                ExtractedPosting {
                    paccount: "Assets:Checking".to_string(),
                    pamount: Some(vec![ExtractedAmount {
                        acommodity: "USD".to_string(),
                        aquantity: "-50.00".to_string(),
                    }]),
                },
                ExtractedPosting {
                    paccount: "Equity:Unreconciled:Venmo".to_string(),
                    pamount: Some(vec![ExtractedAmount {
                        acommodity: "USD".to_string(),
                        aquantity: "50.00".to_string(),
                    }]),
                },
            ]),
        };

        let entry = txn.to_account_entry("Assets:Checking", "Equity:Unreconciled:Checking");
        assert_eq!(entry.postings.len(), 2);
        assert_eq!(entry.postings[0].account, "Assets:Checking");
        assert_eq!(entry.postings[1].account, "Equity:Unreconciled:Venmo");
    }

    #[test]
    fn format_decimal_raw_basic() {
        let raw = crate::hledger::DecimalRaw {
            decimal_places: 2,
            decimal_mantissa: serde_json::Number::from(-2132),
            floating_point: -21.32,
        };
        assert_eq!(format_decimal_raw(&raw), "-21.32");
    }

    #[test]
    fn format_decimal_raw_zero_scale() {
        let raw = crate::hledger::DecimalRaw {
            decimal_places: 0,
            decimal_mantissa: serde_json::Number::from(42),
            floating_point: 42.0,
        };
        assert_eq!(format_decimal_raw(&raw), "42");
    }
}

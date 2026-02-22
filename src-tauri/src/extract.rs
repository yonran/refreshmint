use lopdf::Document as PdfDocument;
use rquickjs::{CatchResultExt, Context, Module, Runtime, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtractScriptContext {
    ledger_dir: String,
    account_name: String,
    extension_name: String,
    document: ExtractDocumentContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    document_info: Option<crate::scrape::DocumentInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    csv: Option<Vec<Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf: Option<PdfExtractContext>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtractDocumentContext {
    name: String,
    path: String,
    format: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PdfExtractContext {
    pages: Vec<PdfPageContext>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PdfPageContext {
    page_number: usize,
    width: f32,
    height: f32,
    text: String,
    items: Vec<PdfTextItemContext>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PdfTextItemContext {
    text: String,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentFormat {
    Csv,
    Pdf,
    Other,
}

impl DocumentFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Pdf => "pdf",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtractionMode<'a> {
    Script(&'a str),
    Rules(&'a str),
}

fn io_error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
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

fn resolve_extraction_mode<'a>(
    extract: Option<&'a str>,
    rules: Option<&'a str>,
) -> Result<ExtractionMode<'a>, String> {
    match (extract, rules) {
        (Some(_), Some(_)) => Err("only one of `extract` or `rules` may be defined".to_string()),
        (None, None) => Err("exactly one of `extract` or `rules` must be defined".to_string()),
        (Some(path), None) => Ok(ExtractionMode::Script(path)),
        (None, Some(path)) => Ok(ExtractionMode::Rules(path)),
    }
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
    let documents_dir = account_journal::account_documents_dir(ledger_dir, account_name);
    run_extraction_with_documents_dir(
        ledger_dir,
        &documents_dir,
        account_name,
        extension_name,
        document_names,
    )
}

/// Run extraction for a login account (`logins/<login>/accounts/<label>`).
pub fn run_extraction_for_login_account(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    account_name: &str,
    extension_name: &str,
    document_names: &[String],
) -> Result<ExtractionResult, Box<dyn std::error::Error + Send + Sync>> {
    let documents_dir = account_journal::login_account_documents_dir(ledger_dir, login_name, label);
    run_extraction_with_documents_dir(
        ledger_dir,
        &documents_dir,
        account_name,
        extension_name,
        document_names,
    )
}

fn run_extraction_with_documents_dir(
    ledger_dir: &Path,
    documents_dir: &Path,
    account_name: &str,
    extension_name: &str,
    document_names: &[String],
) -> Result<ExtractionResult, Box<dyn std::error::Error + Send + Sync>> {
    let extension_dir = crate::account_config::resolve_extension_dir(ledger_dir, extension_name);
    let manifest = crate::scrape::load_manifest(&extension_dir)?;
    let extraction_mode =
        resolve_extraction_mode(manifest.extract.as_deref(), manifest.rules.as_deref()).map_err(
            |err| {
                io_error(format!(
                    "invalid manifest.json for extension '{extension_name}': {err}"
                ))
            },
        )?;

    let mut all_proposed = Vec::new();

    match extraction_mode {
        ExtractionMode::Script(script_rel_path) => {
            let script_path = extension_dir.join(script_rel_path);
            if !script_path.exists() {
                return Err(format!("extract script not found: {}", script_path.display()).into());
            }

            for doc_name in document_names {
                let doc_path = documents_dir.join(doc_name);
                if !doc_path.exists() {
                    return Err(format!("document not found: {}", doc_path.display()).into());
                }
                let proposed = run_extract_script(
                    &script_path,
                    &doc_path,
                    doc_name,
                    documents_dir,
                    ledger_dir,
                    account_name,
                    extension_name,
                )?;
                all_proposed.extend(proposed);
            }
        }
        ExtractionMode::Rules(rules_rel_path) => {
            let rules_path = extension_dir.join(rules_rel_path);
            if !rules_path.exists() {
                return Err(format!("rules file not found: {}", rules_path.display()).into());
            }

            for doc_name in document_names {
                let doc_path = documents_dir.join(doc_name);
                if !doc_path.exists() {
                    return Err(format!("document not found: {}", doc_path.display()).into());
                }
                if !doc_name.to_ascii_lowercase().ends_with(".csv") {
                    return Err(format!(
                        "rules extraction only supports CSV documents, got: {doc_name}"
                    )
                    .into());
                }

                let proposed = run_rules_extraction(
                    &rules_path,
                    &doc_path,
                    doc_name,
                    manifest.id_field.as_deref(),
                )?;
                all_proposed.extend(proposed);
            }
        }
    }

    Ok(ExtractionResult {
        proposed_transactions: all_proposed,
        document_names: document_names.to_vec(),
    })
}

/// Run extract.mjs on a document using QuickJS sandbox.
fn run_extract_script(
    script_path: &Path,
    doc_path: &Path,
    doc_name: &str,
    documents_dir: &Path,
    ledger_dir: &Path,
    account_name: &str,
    extension_name: &str,
) -> Result<Vec<ExtractedTransaction>, Box<dyn std::error::Error + Send + Sync>> {
    let script_source = std::fs::read_to_string(script_path)?;
    let context = build_extract_script_context(
        doc_path,
        doc_name,
        documents_dir,
        ledger_dir,
        account_name,
        extension_name,
    )?;
    let context_json = serde_json::to_string(&context)?;

    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;

    let result_json = context
        .with(|ctx| {
            let module_name = script_path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("extract.mjs");

            let module = Module::declare(ctx.clone(), module_name, script_source.as_str())
                .catch(&ctx)
                .map_err(|error| format!("failed to compile {}: {error}", script_path.display()))?;
            let (module, module_promise) = module.eval().catch(&ctx).map_err(|error| {
                format!("failed to evaluate {}: {error}", script_path.display())
            })?;

            module_promise.finish::<()>().catch(&ctx).map_err(|error| {
                format!(
                    "module initialization failed in {}: {error}",
                    script_path.display()
                )
            })?;

            let extract_export: Value = module.get("extract").catch(&ctx).map_err(|_| {
                format!(
                    "{} must export function `extract(context)`",
                    script_path.display()
                )
            })?;
            let extract_fn = extract_export.into_function().ok_or_else(|| {
                format!(
                    "{} must export function `extract(context)`",
                    script_path.display()
                )
            })?;

            let js_context = ctx
                .json_parse(context_json.as_str())
                .catch(&ctx)
                .map_err(|error| format!("failed to serialize extract context: {error}"))?;

            let returned: Value = extract_fn
                .call((js_context,))
                .catch(&ctx)
                .map_err(|error| format!("extract(context) threw: {error}"))?;

            let resolved = if returned.is_promise() {
                returned
                    .into_promise()
                    .ok_or_else(|| "internal error: promise conversion failed".to_string())?
                    .finish::<Value>()
                    .catch(&ctx)
                    .map_err(|error| format!("extract(context) rejected: {error}"))?
            } else {
                returned
            };

            if !resolved.is_array() {
                return Err("extract(context) must return an array of transactions".to_string());
            }

            ctx.json_stringify(resolved)
                .catch(&ctx)
                .map_err(|error| format!("failed to serialize extractor result: {error}"))?
                .ok_or_else(|| "extract(context) returned a non-serializable value".to_string())?
                .to_string()
                .map_err(|error| format!("failed to decode extractor result: {error}"))
        })
        .map_err(io_error)?;

    let extracted: Vec<ExtractedTransaction> =
        serde_json::from_str(&result_json).map_err(|error| {
            io_error(format!(
                "extract(context) returned invalid transaction JSON: {error}"
            ))
        })?;

    for txn in &extracted {
        validate_extracted_transaction(txn, doc_name)?;
    }

    Ok(extracted)
}

fn build_extract_script_context(
    doc_path: &Path,
    doc_name: &str,
    documents_dir: &Path,
    ledger_dir: &Path,
    account_name: &str,
    extension_name: &str,
) -> Result<ExtractScriptContext, Box<dyn std::error::Error + Send + Sync>> {
    let document_info = read_document_info(documents_dir, doc_name)?;
    let format = detect_document_format(doc_name, document_info.as_ref());

    let csv = match format {
        DocumentFormat::Csv => Some(read_csv_rows(doc_path)?),
        _ => None,
    };
    let pdf = match format {
        DocumentFormat::Pdf => Some(read_pdf_context(doc_path)?),
        _ => None,
    };

    Ok(ExtractScriptContext {
        ledger_dir: ledger_dir.display().to_string(),
        account_name: account_name.to_string(),
        extension_name: extension_name.to_string(),
        document: ExtractDocumentContext {
            name: doc_name.to_string(),
            path: doc_path.display().to_string(),
            format: format.as_str().to_string(),
        },
        document_info,
        csv,
        pdf,
    })
}

fn read_document_info(
    documents_dir: &Path,
    doc_name: &str,
) -> Result<Option<crate::scrape::DocumentInfo>, Box<dyn std::error::Error + Send + Sync>> {
    let sidecar_path = documents_dir.join(format!("{doc_name}-info.json"));
    if !sidecar_path.exists() {
        return Ok(None);
    }

    let sidecar_text = std::fs::read_to_string(&sidecar_path)?;
    let info = serde_json::from_str(&sidecar_text).map_err(|error| {
        io_error(format!(
            "invalid document sidecar {}: {error}",
            sidecar_path.display()
        ))
    })?;
    Ok(Some(info))
}

fn detect_document_format(
    doc_name: &str,
    document_info: Option<&crate::scrape::DocumentInfo>,
) -> DocumentFormat {
    let lower_name = doc_name.to_ascii_lowercase();
    if lower_name.ends_with(".csv") {
        return DocumentFormat::Csv;
    }
    if lower_name.ends_with(".pdf") {
        return DocumentFormat::Pdf;
    }

    if let Some(info) = document_info {
        let mime = info.mime_type.to_ascii_lowercase();
        if mime.contains("csv") {
            return DocumentFormat::Csv;
        }
        if mime.contains("pdf") {
            return DocumentFormat::Pdf;
        }
    }

    DocumentFormat::Other
}

fn read_csv_rows(
    doc_path: &Path,
) -> Result<Vec<Vec<String>>, Box<dyn std::error::Error + Send + Sync>> {
    let bytes = std::fs::read(doc_path)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        io_error(format!(
            "CSV document is not valid UTF-8: {}",
            doc_path.display()
        ))
    })?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(text.as_bytes());

    let mut rows = Vec::new();
    for row in reader.records() {
        let row = row?;
        rows.push(row.iter().map(std::string::ToString::to_string).collect());
    }

    Ok(rows)
}

fn read_pdf_context(
    doc_path: &Path,
) -> Result<PdfExtractContext, Box<dyn std::error::Error + Send + Sync>> {
    let document = PdfDocument::load(doc_path).map_err(|error| {
        io_error(format!(
            "failed to open PDF document {}: {error}",
            doc_path.display()
        ))
    })?;

    let mut pages = Vec::new();
    for (page_number, _object_id) in document.get_pages() {
        let page_text = document.extract_text(&[page_number]).map_err(|error| {
            io_error(format!(
                "failed to read text from PDF page {} in {}: {error}",
                page_number,
                doc_path.display()
            ))
        })?;

        let mut items = Vec::new();
        for (line_index, line) in page_text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }

            // lopdf text extraction does not provide layout geometry.
            // Expose one item per line with synthetic bounds.
            items.push(PdfTextItemContext {
                text: line.to_string(),
                left: 0.0,
                top: line_index as f32,
                width: line.chars().count() as f32,
                height: 1.0,
            });
        }

        let text = page_text.trim().to_string();
        let (width, height) = page_dimensions(&document, page_number);
        pages.push(PdfPageContext {
            page_number: page_number as usize,
            width,
            height,
            text,
            items,
        });
    }

    Ok(PdfExtractContext { pages })
}

fn page_dimensions(document: &PdfDocument, page_number: u32) -> (f32, f32) {
    let Some(rect) = resolve_page_rect(document, page_number, b"CropBox")
        .or_else(|| resolve_page_rect(document, page_number, b"MediaBox"))
    else {
        return (0.0, 0.0);
    };

    let width = (rect[2] - rect[0]).abs();
    let height = (rect[3] - rect[1]).abs();
    (width, height)
}

fn resolve_page_rect(document: &PdfDocument, page_number: u32, key: &[u8]) -> Option<[f32; 4]> {
    let pages = document.get_pages();
    let mut current_id = *pages.get(&page_number)?;
    let mut seen = HashSet::new();

    loop {
        if !seen.insert(current_id) {
            return None;
        }

        let current = document.get_dictionary(current_id).ok()?;
        if let Ok(object) = current.get_deref(key, document) {
            if let Some(rect) = parse_rect(object) {
                return Some(rect);
            }
        }

        current_id = current
            .get(b"Parent")
            .and_then(lopdf::Object::as_reference)
            .ok()?;
    }
}

fn parse_rect(object: &lopdf::Object) -> Option<[f32; 4]> {
    let values = object.as_array().ok()?;
    if values.len() < 4 {
        return None;
    }

    let left = values[0].as_float().ok()?;
    let bottom = values[1].as_float().ok()?;
    let right = values[2].as_float().ok()?;
    let top = values[3].as_float().ok()?;
    Some([left, bottom, right, top])
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
    list_documents_in_dir(&documents_dir)
}

/// List evidence documents for a login account.
pub fn list_documents_for_login_account(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
) -> io::Result<Vec<DocumentWithInfo>> {
    let documents_dir = account_journal::login_account_documents_dir(ledger_dir, login_name, label);
    list_documents_in_dir(&documents_dir)
}

/// Read raw CSV rows from a document in a login account's documents directory.
pub fn read_login_account_document_csv_rows(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    document_name: &str,
) -> Result<Vec<Vec<String>>, Box<dyn std::error::Error + Send + Sync>> {
    let documents_dir = account_journal::login_account_documents_dir(ledger_dir, login_name, label);
    let doc_path = documents_dir.join(document_name);
    read_csv_rows(&doc_path)
}

fn list_documents_in_dir(documents_dir: &Path) -> io::Result<Vec<DocumentWithInfo>> {
    if !documents_dir.exists() {
        return Ok(Vec::new());
    }

    let mut documents = Vec::new();
    for entry in std::fs::read_dir(documents_dir)? {
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
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("refreshmint-{prefix}-{}-{now}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

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

    #[test]
    fn resolve_extraction_mode_rejects_both_extract_and_rules() {
        let err = resolve_extraction_mode(Some("extract.mjs"), Some("account.rules"))
            .expect_err("expected mode conflict");
        assert!(err.contains("only one of `extract` or `rules`"));
    }

    #[test]
    fn resolve_extraction_mode_rejects_missing_extract_and_rules() {
        let err = resolve_extraction_mode(None, None).expect_err("expected missing mode");
        assert!(err.contains("exactly one of `extract` or `rules`"));
    }

    #[test]
    fn run_extract_script_executes_async_extract_function() {
        let root = temp_dir("extract-script-ok");
        let documents_dir = root.join("documents");
        fs::create_dir_all(&documents_dir).expect("create docs dir");

        let script_path = root.join("extract.mjs");
        fs::write(
            &script_path,
            r#"
export async function extract(context) {
  if (!Array.isArray(context.csv) || context.csv.length !== 2) {
    throw new Error("unexpected csv shape");
  }
  return [{
    tdate: context.csv[1][0],
    tstatus: "Cleared",
    tdescription: context.csv[1][1],
    tcomment: "",
    ttags: [
      ["evidence", `${context.document.name}:2:1`],
      ["bankId", context.csv[1][2]]
    ]
  }];
}
"#,
        )
        .expect("write extract script");

        let doc_name = "statement.csv";
        let doc_path = documents_dir.join(doc_name);
        fs::write(
            &doc_path,
            "date,description,bank_id\n2024-01-05,Coffee Shop,fit-123\n",
        )
        .expect("write csv document");

        let txns = run_extract_script(
            &script_path,
            &doc_path,
            doc_name,
            &documents_dir,
            &root,
            "Assets:Checking",
            "example-extension",
        )
        .expect("extract script should succeed");

        assert_eq!(txns.len(), 1);
        assert_eq!(txns[0].tdate, "2024-01-05");
        assert_eq!(txns[0].tdescription, "Coffee Shop");
        assert_eq!(txns[0].bank_id(), Some("fit-123"));
    }

    #[test]
    fn run_extract_script_requires_extract_export() {
        let root = temp_dir("extract-script-missing-export");
        let documents_dir = root.join("documents");
        fs::create_dir_all(&documents_dir).expect("create docs dir");

        let script_path = root.join("extract.mjs");
        fs::write(&script_path, "export const version = '1.0.0';\n").expect("write script");

        let doc_name = "statement.csv";
        let doc_path = documents_dir.join(doc_name);
        fs::write(&doc_path, "date,description\n2024-01-05,Coffee Shop\n")
            .expect("write csv document");

        let err = run_extract_script(
            &script_path,
            &doc_path,
            doc_name,
            &documents_dir,
            &root,
            "Assets:Checking",
            "example-extension",
        )
        .expect_err("expected missing export error");

        assert!(err
            .to_string()
            .contains("must export function `extract(context)`"));
    }

    #[test]
    fn run_extract_script_requires_array_result() {
        let root = temp_dir("extract-script-bad-result");
        let documents_dir = root.join("documents");
        fs::create_dir_all(&documents_dir).expect("create docs dir");

        let script_path = root.join("extract.mjs");
        fs::write(
            &script_path,
            r#"
export function extract(_context) {
  return { ok: true };
}
"#,
        )
        .expect("write script");

        let doc_name = "statement.csv";
        let doc_path = documents_dir.join(doc_name);
        fs::write(&doc_path, "date,description\n2024-01-05,Coffee Shop\n")
            .expect("write csv document");

        let err = run_extract_script(
            &script_path,
            &doc_path,
            doc_name,
            &documents_dir,
            &root,
            "Assets:Checking",
            "example-extension",
        )
        .expect_err("expected non-array result error");

        assert!(err
            .to_string()
            .contains("extract(context) must return an array of transactions"));
    }
}

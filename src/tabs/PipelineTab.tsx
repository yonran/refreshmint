import {
    Fragment,
    useCallback,
    useEffect,
    useMemo,
    useRef,
    useState,
} from 'react';
import { listen } from '@tauri-apps/api/event';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import {
    type AccountJournalEntry,
    type CategoryResult,
    getLoginAccountJournal,
    getLoginAccountUnposted,
    getLoginExtractionSupport,
    getLockStatusSnapshot,
    getUnpostedEntriesForTransfer,
    applyAutomationProposal,
    createResolution,
    disableResolution,
    enableResolution,
    applyAutomationPolicy,
    type AutomationScope,
    listAutomationProposals,
    normalizePayee,
    type AutomationProposal,
    listImportAnomalies,
    listResolutions,
    type Resolution,
    type LoginConfig,
    type LockStatusSnapshot,
    type ImportAnomaly,
    listLoginAccountDocuments,
    type DocumentWithInfo,
    type LedgerView,
    postLoginAccountEntry,
    readAttachmentDataUrl,
    readLoginAccountDocumentRows,
    readLoginAccountDocumentText,
    postLoginAccountEntrySplit,
    postLoginAccountTransfer,
    retireLoginAccountEntry,
    runLoginAccountExtraction,
    setLoginAccount,
    startLockMetadataWatch,
    stopLockMetadataWatch,
    suggestCategories,
    syncGlTransaction,
    reviewImportAnomaly,
    type UnpostedTransferResult,
    type TypedRef,
} from '../tauri-commands.ts';
import {
    createTransferLinkResolution,
    loginEntryRef,
    resolutionInputFromProposal,
} from '../automation-utils.ts';
import {
    type LoginAccountRef,
    type PipelineSubTab,
    type PipelineBulkAccountStat,
    type PipelineBulkStats,
    type PipelineTabSession,
    type SplitDraftRow,
    createEmptyPipelineBulkSummary,
    normalizeLoginConfig,
    suggestGlAccountName,
} from '../types.ts';
import { AccountInput } from './TransactionsTable.tsx';
import { TRANSFER_CANCEL_EPSILON } from '../gl-transfer-utils.ts';

interface PipelineTabProps {
    ledger: LedgerView;
    isActive: boolean;
    loginAccounts: LoginAccountRef[];
    loginConfigsByName: Record<string, LoginConfig>;
    hasLoadedLoginConfigs: boolean;
    onLedgerRefresh: () => void;
    onLoginConfigChanged: () => void;
    onViewGlTransaction: (glTxnId: string) => void;
    session: PipelineTabSession;
    onSessionChange: (
        updater: (current: PipelineTabSession) => PipelineTabSession,
    ) => void;
}

// Login-account scope ref for a CategoryRule (no entryId — matches the Rust
// automation::is_login_scope_ref check). Restricts a rule to one bank account.
function loginScopeRef(loginName: string, label: string): TypedRef {
    return {
        kind: 'login-entry',
        locator: `logins/${loginName}/accounts/${label}`,
        loginName,
        label,
    };
}

function evidenceRef(ref: string): TypedRef {
    return {
        kind: 'evidence-row',
        locator: ref,
        id: ref,
    };
}

function refLabel(ref: TypedRef): string {
    if (ref.kind === 'login-entry') {
        return `${ref.loginName ?? '?'}:${ref.label ?? '?'}:${ref.entryId ?? ref.id ?? '?'}`;
    }
    return ref.locator ?? ref.filename ?? ref.id ?? ref.kind;
}

function resolutionSubjectLabel(resolution: Resolution): string {
    return resolution.subjectRefs.map(refLabel).join(' <-> ');
}

function resolutionResultLabel(resolution: Resolution): string {
    if (resolution.parts.length > 0) {
        return resolution.parts
            .map((part) =>
                `${part.account ?? refLabel(part.ref ?? { kind: 'document' })} ${part.amount ?? ''}`.trim(),
            )
            .join(' + ');
    }
    return resolutionSubjectLabel(resolution);
}

function filterLoginAccountResolutions(
    resolutions: Resolution[],
    loginName: string,
    label: string,
): Resolution[] {
    return resolutions.filter((resolution) =>
        resolution.subjectRefs.some(
            (ref) =>
                ref.kind === 'login-entry' &&
                ref.loginName === loginName &&
                ref.label === label,
        ),
    );
}

export function PipelineTab({
    ledger,
    isActive,
    loginAccounts,
    loginConfigsByName,
    hasLoadedLoginConfigs,
    onLedgerRefresh,
    onLoginConfigChanged,
    onViewGlTransaction,
    session,
    onSessionChange,
}: PipelineTabProps) {
    const ledgerPath = ledger.path;

    const [selectedLoginAccount, setSelectedLoginAccount] =
        useState<LoginAccountRef | null>(session.selectedLoginAccount);
    const [documents, setDocuments] = useState<DocumentWithInfo[]>([]);
    const [accountJournalEntries, setAccountJournalEntries] = useState<
        AccountJournalEntry[]
    >([]);
    const [unpostedEntries, setUnpostedEntries] = useState<
        AccountJournalEntry[]
    >([]);
    const [pipelineStatus, setPipelineStatus] = useState<string | null>(
        session.pipelineStatus,
    );
    const [pipelineSubTab, setPipelineSubTab] = useState<PipelineSubTab>(
        session.pipelineSubTab,
    );
    const [evidenceRowsDocument, setEvidenceRowsDocument] = useState(
        session.evidenceRowsDocument,
    );
    const [documentRows, setDocumentRows] = useState<string[][]>([]);
    const [isLoadingDocumentRows, setIsLoadingDocumentRows] = useState(false);
    const [isLoadingDocuments, setIsLoadingDocuments] = useState(false);
    const [lightboxSrc, setLightboxSrc] = useState<string | null>(null);
    const [lightboxFilename, setLightboxFilename] = useState<string | null>(
        null,
    );
    const [lightboxLoading, setLightboxLoading] = useState(false);
    const [lightboxError, setLightboxError] = useState<string | null>(null);
    const [textViewContent, setTextViewContent] = useState<string | null>(null);
    const [textViewFilename, setTextViewFilename] = useState<string | null>(
        null,
    );
    const [isRunningExtraction, setIsRunningExtraction] = useState(false);
    const [isLoadingAccountJournal, setIsLoadingAccountJournal] =
        useState(false);
    const [busyPostEntryId, setBusyPostEntryId] = useState<string | null>(null);
    const [pipelineSelectedEntryIds, setPipelineSelectedEntryIds] = useState<
        Set<string>
    >(new Set(session.pipelineSelectedEntryIds));
    const [pipelineCategorySuggestions, setPipelineCategorySuggestions] =
        useState<Record<string, CategoryResult>>({});
    const [importAnomalies, setImportAnomalies] = useState<ImportAnomaly[]>([]);
    const [automationProposals, setAutomationProposals] = useState<
        AutomationProposal[]
    >([]);
    const [activeResolutions, setActiveResolutions] = useState<Resolution[]>(
        [],
    );
    const [busyProposalId, setBusyProposalId] = useState<string | null>(null);
    const [busyAnomalyId, setBusyAnomalyId] = useState<string | null>(null);
    const [busyResolutionId, setBusyResolutionId] = useState<string | null>(
        null,
    );
    const [pipelineGlAccountDraft, setPipelineGlAccountDraft] = useState(
        session.pipelineGlAccountDraft,
    );
    const [isSavingPipelineGlAccount, setIsSavingPipelineGlAccount] =
        useState(false);
    const [isPipelinePosting, setIsPipelinePosting] = useState(false);
    const [isPipelineExtractingAllLedger, setIsPipelineExtractingAllLedger] =
        useState(false);
    const [isPipelinePostingAllLedger, setIsPipelinePostingAllLedger] =
        useState(false);
    const [lockStatusSnapshot, setLockStatusSnapshot] =
        useState<LockStatusSnapshot | null>(null);
    const [pipelineBulkStats, setPipelineBulkStats] =
        useState<PipelineBulkStats | null>(null);
    const [isLoadingPipelineBulkStats, setIsLoadingPipelineBulkStats] =
        useState(false);
    const [transferModalEntryId, setTransferModalEntryId] = useState<
        string | null
    >(session.transferModalEntryId);
    const [splitModalEntryId, setSplitModalEntryId] = useState<string | null>(
        session.splitModalEntryId,
    );
    const [splitDraftRows, setSplitDraftRows] = useState<SplitDraftRow[]>(
        session.splitDraftRows,
    );
    const [transferModalResults, setTransferModalResults] = useState<
        UnpostedTransferResult[]
    >([]);
    const [isLoadingTransferModal, setIsLoadingTransferModal] = useState(false);
    const [transferModalSearch, setTransferModalSearch] = useState(
        session.transferModalSearch,
    );
    // Fee prompt inside the Link Transfer modal: set when the picked candidate
    // does not cancel the subject amount. The post is held until the user
    // confirms a fee account. Mirrors the Transactions modal fee prompt.
    const [transferModalFeePrompt, setTransferModalFeePrompt] = useState<{
        candidate: UnpostedTransferResult;
        residual: string;
    } | null>(null);
    const [transferModalFeeAccount, setTransferModalFeeAccount] =
        useState('Expenses:Bank Fees');

    const suggestRequestId = useRef(0);
    const hasSeenSelectedLoginAccountRef = useRef(false);
    const sessionRef = useRef<PipelineTabSession>(session);

    // --- Computed values ---

    const hasExtension =
        selectedLoginAccount !== null &&
        (loginConfigsByName[
            selectedLoginAccount.loginName
        ]?.extension?.trim() ?? '') !== '';

    const refreshPipelineBulkStats = useCallback(async () => {
        setIsLoadingPipelineBulkStats(true);
        try {
            const uniqueLoginNames = Array.from(
                new Set(loginAccounts.map((account) => account.loginName)),
            );
            const snapshot = await getLockStatusSnapshot(
                ledgerPath,
                uniqueLoginNames,
            );
            const extractionSupportEntries = await Promise.all(
                uniqueLoginNames.map(
                    async (loginName) =>
                        [
                            loginName,
                            await getLoginExtractionSupport(
                                ledgerPath,
                                loginName,
                            ),
                        ] as const,
                ),
            );
            const extractionSupportByLogin = new Map(extractionSupportEntries);
            const accountStats = await Promise.all(
                loginAccounts.map(async ({ loginName, label }) => {
                    const normalizedConfig = normalizeLoginConfig(
                        loginConfigsByName[loginName] ?? null,
                    );
                    const extension = normalizedConfig.extension?.trim() ?? '';
                    const extractionSupport =
                        extractionSupportByLogin.get(loginName) ?? null;
                    const glAccount =
                        normalizedConfig.accounts[label]?.glAccount?.trim() ??
                        '';
                    const locked = snapshot.logins[loginName]?.locked ?? false;

                    let documentCount = 0;
                    let extractSkipReason:
                        | 'missing-extension'
                        | 'missing-extractor'
                        | 'broken-extractor'
                        | 'no-documents'
                        | null = null;
                    let extractInspectError: string | null = null;
                    if (
                        extension.length === 0 ||
                        extractionSupport?.reason === 'missing-extension'
                    ) {
                        extractSkipReason = 'missing-extension';
                    } else if (
                        extractionSupport?.reason === 'missing-extractor'
                    ) {
                        extractSkipReason = 'missing-extractor';
                    } else if (
                        extractionSupport?.reason === 'broken-extractor'
                    ) {
                        extractSkipReason = 'broken-extractor';
                    } else {
                        try {
                            const docs = await listLoginAccountDocuments(
                                ledgerPath,
                                loginName,
                                label,
                            );
                            documentCount = docs.length;
                            if (documentCount === 0) {
                                extractSkipReason = 'no-documents';
                            }
                        } catch (error) {
                            extractInspectError = String(error);
                        }
                    }

                    let unpostedCount = 0;
                    let postSkipReason:
                        | 'missing-gl-account'
                        | 'no-unposted'
                        | null = null;
                    let postInspectError: string | null = null;
                    if (glAccount.length === 0) {
                        postSkipReason = 'missing-gl-account';
                    } else {
                        try {
                            const unposted = await getLoginAccountUnposted(
                                ledgerPath,
                                loginName,
                                label,
                            );
                            unpostedCount = unposted.length;
                            if (unpostedCount === 0) {
                                postSkipReason = 'no-unposted';
                            }
                        } catch (error) {
                            postInspectError = String(error);
                        }
                    }

                    return {
                        loginName,
                        label,
                        extract: {
                            eligible:
                                extractSkipReason === null &&
                                extractInspectError === null &&
                                documentCount > 0,
                            documentCount,
                            skipReason: extractSkipReason,
                            inspectError: extractInspectError,
                            locked,
                        },
                        post: {
                            eligible:
                                postSkipReason === null &&
                                postInspectError === null &&
                                unpostedCount > 0,
                            unpostedCount,
                            skipReason: postSkipReason,
                            inspectError: postInspectError,
                            locked,
                        },
                    } satisfies PipelineBulkAccountStat;
                }),
            );

            const extract = createEmptyPipelineBulkSummary();
            const post = createEmptyPipelineBulkSummary();
            for (const account of accountStats) {
                if (account.extract.eligible) {
                    extract.eligibleAccounts += 1;
                    extract.totalDocuments += account.extract.documentCount;
                }
                if (account.extract.skipReason === 'missing-extension') {
                    extract.skippedMissingExtension += 1;
                }
                if (
                    account.extract.skipReason === 'missing-extractor' ||
                    account.extract.skipReason === 'broken-extractor'
                ) {
                    extract.skippedMissingExtractor += 1;
                }
                if (account.extract.skipReason === 'no-documents') {
                    extract.skippedNoDocuments += 1;
                }
                if (account.extract.inspectError !== null) {
                    extract.inspectFailures += 1;
                }
                if (account.extract.locked) {
                    extract.lockedAccounts += 1;
                }

                if (account.post.eligible) {
                    post.eligibleAccounts += 1;
                    post.totalUnpostedEntries += account.post.unpostedCount;
                }
                if (account.post.skipReason === 'missing-gl-account') {
                    post.skippedMissingGlAccount += 1;
                }
                if (account.post.skipReason === 'no-unposted') {
                    post.skippedNoUnposted += 1;
                }
                if (account.post.inspectError !== null) {
                    post.inspectFailures += 1;
                }
                if (account.post.locked) {
                    post.lockedAccounts += 1;
                }
            }

            const stats = {
                accounts: accountStats,
                gl: snapshot.gl,
                extract,
                post,
            } satisfies PipelineBulkStats;
            setLockStatusSnapshot(snapshot);
            setPipelineBulkStats(stats);
            return stats;
        } finally {
            setIsLoadingPipelineBulkStats(false);
        }
    }, [ledgerPath, loginAccounts, loginConfigsByName]);

    const pipelineGlAccount = useMemo<string | null>(() => {
        if (!selectedLoginAccount) return null;
        return (
            loginConfigsByName[selectedLoginAccount.loginName]?.accounts[
                selectedLoginAccount.label
            ]?.glAccount ?? null
        );
    }, [selectedLoginAccount, loginConfigsByName]);

    const glLockStatus = lockStatusSnapshot?.gl ?? {
        locked: false,
        metadata: null,
    };
    const selectedLoginLockStatus =
        selectedLoginAccount === null
            ? null
            : (lockStatusSnapshot?.logins[selectedLoginAccount.loginName] ??
              null);
    const selectedLoginLocked = selectedLoginLockStatus?.locked ?? false;

    const visibleTransferResults = useMemo(() => {
        const q = transferModalSearch.trim().toLowerCase();
        if (!q) return transferModalResults;
        return transferModalResults.filter(
            (r) =>
                r.loginName.toLowerCase().includes(q) ||
                r.label.toLowerCase().includes(q) ||
                r.entry.description.toLowerCase().includes(q) ||
                r.entry.date.includes(q) ||
                (r.entry.amount ?? '').includes(q),
        );
    }, [transferModalResults, transferModalSearch]);

    const evidencedRowNumbers = useMemo(() => {
        if (!evidenceRowsDocument) return new Set<number>();
        const rowNumbers = new Set<number>();
        for (const entry of accountJournalEntries) {
            for (const evidence of entry.evidence) {
                const prefix = evidenceRowsDocument + ':';
                if (evidence.startsWith(prefix)) {
                    const rest = evidence.slice(prefix.length);
                    const rowNum = parseInt(rest.split(':')[0] ?? '', 10);
                    if (!isNaN(rowNum)) {
                        rowNumbers.add(rowNum);
                    }
                }
            }
        }
        return rowNumbers;
    }, [evidenceRowsDocument, accountJournalEntries]);

    // --- Effects ---

    useEffect(() => {
        sessionRef.current = {
            selectedLoginAccount,
            pipelineStatus,
            pipelineSubTab,
            evidenceRowsDocument,
            pipelineSelectedEntryIds: [...pipelineSelectedEntryIds],
            pipelineGlAccountDraft,
            transferModalEntryId,
            splitModalEntryId,
            splitDraftRows,
            transferModalSearch,
        };
    }, [
        selectedLoginAccount,
        pipelineStatus,
        pipelineSubTab,
        evidenceRowsDocument,
        pipelineSelectedEntryIds,
        pipelineGlAccountDraft,
        transferModalEntryId,
        splitModalEntryId,
        splitDraftRows,
        transferModalSearch,
    ]);

    // Persist the latest local tab state when the tab unmounts.
    useEffect(() => {
        return () => {
            onSessionChange(() => sessionRef.current);
        };
    }, [onSessionChange]);

    // Auto-select the first login account whenever the account list changes.
    useEffect(() => {
        if (loginAccounts.length === 0) return;
        setSelectedLoginAccount((current) => {
            if (
                current !== null &&
                loginAccounts.some(
                    (a) =>
                        a.loginName === current.loginName &&
                        a.label === current.label,
                )
            ) {
                return current;
            }
            return loginAccounts[0] ?? null;
        });
    }, [loginAccounts]);

    // Reset per-account UI state when the selected account changes.
    useEffect(() => {
        if (!hasSeenSelectedLoginAccountRef.current) {
            hasSeenSelectedLoginAccountRef.current = true;
            return;
        }
        setEvidenceRowsDocument('');
        setDocumentRows([]);
        setPipelineSelectedEntryIds(new Set());
        setPipelineCategorySuggestions({});
        setPipelineGlAccountDraft(
            selectedLoginAccount
                ? suggestGlAccountName(selectedLoginAccount.label)
                : '',
        );
        setTransferModalEntryId(null);
    }, [selectedLoginAccount]);

    // Watch for lock-status changes and refresh bulk stats.
    useEffect(() => {
        let cancelled = false;
        let unlisten: (() => void) | null = null;

        void startLockMetadataWatch(ledgerPath)
            .then(() =>
                listen('refreshmint://lock-status-changed', () => {
                    if (!cancelled) {
                        void refreshPipelineBulkStats();
                    }
                }),
            )
            .then((listener) => {
                if (!cancelled) {
                    unlisten = listener;
                } else {
                    listener();
                }
            })
            .catch((error: unknown) => {
                console.error('lock metadata watcher failed:', error);
            });

        return () => {
            cancelled = true;
            if (unlisten !== null) {
                unlisten();
            }
            void stopLockMetadataWatch();
        };
    }, [ledgerPath, refreshPipelineBulkStats]);

    // Refresh bulk stats when the tab becomes active.
    useEffect(() => {
        if (!isActive || !hasLoadedLoginConfigs) {
            return;
        }
        void refreshPipelineBulkStats().catch((error: unknown) => {
            console.error('pipeline bulk stats failed:', error);
        });
    }, [
        isActive,
        hasLoadedLoginConfigs,
        loginAccounts,
        loginConfigsByName,
        refreshPipelineBulkStats,
    ]);

    // Load documents and journal entries for the selected login account.
    useEffect(() => {
        if (selectedLoginAccount === null) {
            setActiveResolutions([]);
            setAutomationProposals([]);
            setImportAnomalies([]);
            return;
        }

        const { loginName, label } = selectedLoginAccount;
        let cancelled = false;
        const timer = window.setTimeout(() => {
            setIsLoadingDocuments(true);
            setIsLoadingAccountJournal(true);
            void Promise.all([
                listLoginAccountDocuments(ledgerPath, loginName, label),
                getLoginAccountJournal(ledgerPath, loginName, label),
                getLoginAccountUnposted(ledgerPath, loginName, label),
                listImportAnomalies(ledgerPath),
                listAutomationProposals(ledgerPath, {
                    loginName,
                    label,
                    includeGl: false,
                }),
                listResolutions(ledgerPath),
            ])
                .then(
                    ([
                        fetchedDocuments,
                        fetchedJournal,
                        fetchedUnposted,
                        anomalies,
                        proposals,
                        resolutions,
                    ]) => {
                        if (cancelled) {
                            return;
                        }
                        setDocuments(fetchedDocuments);
                        setAccountJournalEntries(fetchedJournal);
                        setUnpostedEntries(fetchedUnposted);
                        setAutomationProposals(proposals);
                        setActiveResolutions(
                            filterLoginAccountResolutions(
                                resolutions,
                                loginName,
                                label,
                            ),
                        );
                        setImportAnomalies(
                            anomalies.filter(
                                (anomaly) =>
                                    anomaly.status === 'open' &&
                                    anomaly.loginName === loginName &&
                                    anomaly.label === label,
                            ),
                        );
                        // Non-blocking category suggestions (fail-open)
                        const reqId = ++suggestRequestId.current;
                        suggestCategories(ledgerPath, loginName, label)
                            .then((result) => {
                                if (
                                    !cancelled &&
                                    reqId === suggestRequestId.current
                                ) {
                                    setPipelineCategorySuggestions(result);
                                }
                            })
                            .catch((err: unknown) => {
                                console.error('suggestCategories failed:', err);
                            });
                    },
                )
                .catch((error: unknown) => {
                    if (!cancelled) {
                        setPipelineStatus(
                            `Failed to load login account journal: ${String(error)}`,
                        );
                    }
                })
                .finally(() => {
                    if (!cancelled) {
                        setIsLoadingDocuments(false);
                        setIsLoadingAccountJournal(false);
                    }
                });
        }, 200);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [ledgerPath, selectedLoginAccount]);

    // --- Handlers ---

    // Same fixed set as TransactionsTable.tsx — must stay in sync with
    // read_attachment_data_url in src-tauri/src/extract.rs:image_mime_type().
    const IMAGE_EXTENSIONS = ['.jpg', '.jpeg', '.png', '.gif', '.webp'];
    function isImageDocument(filename: string): boolean {
        return IMAGE_EXTENSIONS.some((ext) =>
            filename.toLowerCase().endsWith(ext),
        );
    }
    function isCsvDocument(filename: string): boolean {
        return filename.toLowerCase().endsWith('.csv');
    }
    function isPdfDocument(filename: string): boolean {
        return filename.toLowerCase().endsWith('.pdf');
    }
    function closeLightbox() {
        setLightboxSrc(null);
        setLightboxFilename(null);
        setLightboxLoading(false);
        setLightboxError(null);
    }
    async function handleDocumentView(doc: DocumentWithInfo) {
        if (isImageDocument(doc.filename)) {
            setLightboxFilename(doc.filename);
            setLightboxSrc(null);
            setLightboxError(null);
            setLightboxLoading(true);
            try {
                const src = await readAttachmentDataUrl(
                    ledgerPath,
                    doc.filename,
                );
                setLightboxSrc(src);
            } catch (e) {
                setLightboxError(String(e));
            } finally {
                setLightboxLoading(false);
            }
        } else if (isCsvDocument(doc.filename)) {
            await handleLoadDocumentRows(doc.filename);
            setPipelineSubTab('evidence-rows');
        } else {
            // OFX, HTML, JSON, TXT, XML, etc. — show as raw text
            if (selectedLoginAccount === null) return;
            try {
                const text = await readLoginAccountDocumentText(
                    ledgerPath,
                    selectedLoginAccount.loginName,
                    selectedLoginAccount.label,
                    doc.filename,
                );
                setTextViewFilename(doc.filename);
                setTextViewContent(text);
            } catch (e) {
                setTextViewFilename(doc.filename);
                setTextViewContent(`Error reading file: ${String(e)}`);
            }
        }
    }

    async function handleLoadDocumentRows(documentName: string) {
        setEvidenceRowsDocument(documentName);
        if (!documentName || !selectedLoginAccount) {
            setDocumentRows([]);
            return;
        }
        setIsLoadingDocumentRows(true);
        try {
            const rows = await readLoginAccountDocumentRows(
                ledgerPath,
                selectedLoginAccount.loginName,
                selectedLoginAccount.label,
                documentName,
            );
            setDocumentRows(rows);
        } catch {
            setDocumentRows([]);
        } finally {
            setIsLoadingDocumentRows(false);
        }
    }

    async function handlePipelineExtraction(documentName: string) {
        if (selectedLoginAccount === null || !documentName) return;
        const { loginName, label } = selectedLoginAccount;
        setIsRunningExtraction(true);
        setPipelineStatus(`Running extraction for ${documentName}...`);
        try {
            const result = await runLoginAccountExtraction(
                ledgerPath,
                loginName,
                label,
                [documentName],
            );
            const newCount = result.newEntryCount;
            const [journal, unposted, anomalies, proposals, resolutions] =
                await Promise.all([
                    getLoginAccountJournal(ledgerPath, loginName, label),
                    getLoginAccountUnposted(ledgerPath, loginName, label),
                    listImportAnomalies(ledgerPath),
                    listAutomationProposals(ledgerPath, {
                        loginName,
                        label,
                        includeGl: false,
                    }),
                    listResolutions(ledgerPath),
                ]);
            setAccountJournalEntries(journal);
            setUnpostedEntries(unposted);
            setAutomationProposals(proposals);
            setActiveResolutions(
                filterLoginAccountResolutions(resolutions, loginName, label),
            );
            setImportAnomalies(
                anomalies.filter(
                    (anomaly) =>
                        anomaly.status === 'open' &&
                        anomaly.loginName === loginName &&
                        anomaly.label === label,
                ),
            );
            setPipelineStatus(
                `Extraction complete. ${newCount} new entr${newCount === 1 ? 'y' : 'ies'} added.`,
            );
            void refreshPipelineBulkStats();
        } catch (error) {
            setPipelineStatus(`Extraction failed: ${String(error)}`);
        } finally {
            setIsRunningExtraction(false);
        }
    }

    /** Reload journal + unposted data for the current pipeline login/label. */
    async function refreshPipelineLoginAccountData() {
        if (!selectedLoginAccount) return;
        const { loginName, label } = selectedLoginAccount;
        const [
            fetchedJournal,
            fetchedUnposted,
            anomalies,
            proposals,
            resolutions,
        ] = await Promise.all([
            getLoginAccountJournal(ledgerPath, loginName, label),
            getLoginAccountUnposted(ledgerPath, loginName, label),
            listImportAnomalies(ledgerPath),
            listAutomationProposals(ledgerPath, {
                loginName,
                label,
                includeGl: false,
            }),
            listResolutions(ledgerPath),
        ]);
        setAccountJournalEntries(fetchedJournal);
        setUnpostedEntries(fetchedUnposted);
        setAutomationProposals(proposals);
        setActiveResolutions(
            filterLoginAccountResolutions(resolutions, loginName, label),
        );
        setImportAnomalies(
            anomalies.filter(
                (anomaly) =>
                    anomaly.status === 'open' &&
                    anomaly.loginName === loginName &&
                    anomaly.label === label,
            ),
        );
        // Reload full ledger so Transactions tab and GL Rows stay in sync.
        onLedgerRefresh();
        // Non-blocking re-run of suggestCategories to refresh mismatch flags
        const reqId = ++suggestRequestId.current;
        suggestCategories(ledgerPath, loginName, label)
            .then((result) => {
                if (reqId === suggestRequestId.current) {
                    setPipelineCategorySuggestions(result);
                }
            })
            .catch((err: unknown) => {
                console.error('suggestCategories failed:', err);
            });
    }

    async function handleSavePipelineGlAccount() {
        if (!selectedLoginAccount) return;
        const { loginName, label } = selectedLoginAccount;
        const glAccount = pipelineGlAccountDraft.trim();
        setIsSavingPipelineGlAccount(true);
        try {
            await setLoginAccount(
                ledgerPath,
                loginName,
                label,
                glAccount.length === 0 ? null : glAccount,
            );
            onLoginConfigChanged();
            setPipelineGlAccountDraft('');
            void refreshPipelineBulkStats();
        } catch (error) {
            setPipelineStatus(`Failed to save GL account: ${String(error)}`);
        } finally {
            setIsSavingPipelineGlAccount(false);
        }
    }

    async function doPipelinePostForAccount(
        loginName: string,
        label: string,
        entryId: string,
        suggestions: Record<string, CategoryResult>,
    ): Promise<string> {
        const suggestion = suggestions[entryId];

        if (suggestion?.transferMatch) {
            const locator = suggestion.transferMatch.accountLocator;
            const parts = locator.split('/');
            const otherLoginName = parts[1] ?? '';
            const otherLabel = parts[3] ?? '';
            if (!otherLoginName || !otherLabel) {
                throw new Error(
                    `Transfer: could not parse locator: ${locator}`,
                );
            }
            await createTransferLinkResolution(
                ledgerPath,
                { loginName, label, entryId },
                {
                    loginName: otherLoginName,
                    label: otherLabel,
                    entryId: suggestion.transferMatch.entryId,
                },
            );
            const glId = await postLoginAccountTransfer(
                ledgerPath,
                loginName,
                label,
                entryId,
                otherLoginName,
                otherLabel,
                suggestion.transferMatch.entryId,
            );
            return `Transfer posted: ${entryId} ↔ ${suggestion.transferMatch.entryId} (${glId})`;
        }

        // A matching CategoryRule posts directly to its account (one GL write);
        // else Expenses:Unknown. Mirrors App.tsx auto-ETL / cli.rs
        // post_all_counterpart (categorize CategoryResult.ruleAccount).
        const glId = await postLoginAccountEntry(
            ledgerPath,
            loginName,
            label,
            entryId,
            suggestion?.ruleAccount ?? 'Expenses:Unknown',
            null,
        );
        return `Posted ${entryId} to ${glId}`;
    }

    /**
     * Core posting logic that throws on error. Used by both the single-entry
     * handler (which catches) and the bulk handlers.
     */
    async function doPipelinePost(entryId: string): Promise<string> {
        if (!selectedLoginAccount) {
            throw new Error('No account selected.');
        }
        return doPipelinePostForAccount(
            selectedLoginAccount.loginName,
            selectedLoginAccount.label,
            entryId,
            pipelineCategorySuggestions,
        );
    }

    async function handlePipelinePostSplit(
        entryId: string,
        rows: SplitDraftRow[],
    ) {
        if (!selectedLoginAccount) return;
        if (rows.length < 2) {
            setPipelineStatus(
                'Split requires at least 2 counterpart accounts.',
            );
            return;
        }
        if (rows.some((r) => r.account.trim() === '')) {
            setPipelineStatus('All counterpart accounts must be non-empty.');
            return;
        }
        const { loginName, label } = selectedLoginAccount;
        setBusyPostEntryId(entryId);
        setSplitModalEntryId(null);
        try {
            await createResolution(ledgerPath, {
                kind: 'posting-split',
                subjectRefs: [loginEntryRef(loginName, label, entryId)],
                parts: rows.map((r) => ({
                    account: r.account.trim(),
                    amount: r.amount.trim() || null,
                    ref: null,
                    notes: null,
                })),
                notes: 'Created from Pipeline split post',
            });
            const glId = await postLoginAccountEntrySplit(
                ledgerPath,
                loginName,
                label,
                entryId,
                rows.map((r) => ({
                    account: r.account.trim(),
                    amount: r.amount.trim() || null,
                })),
            );
            await refreshPipelineLoginAccountData();
            setPipelineStatus(`Split posted ${entryId} to ${glId}.`);
            void refreshPipelineBulkStats();
        } catch (error) {
            setPipelineStatus(`Split post failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
        }
    }

    // "Always <account>": create a standing CategoryRule keyed on the entry's
    // normalized payee (scoped to this login/account), superseding the old
    // entry-bound Category resolution. Rules fire repeatedly and post future
    // matches directly (see the direct-post + policy-loop paths).
    async function handleCreateCategoryResolution(
        entry: AccountJournalEntry,
        account: string,
    ) {
        if (!selectedLoginAccount) return;
        const trimmed = account.trim();
        if (!trimmed) return;
        const { loginName, label } = selectedLoginAccount;
        setBusyPostEntryId(entry.id);
        try {
            // Confirm against the NORMALIZED payee (what the rule actually keys
            // on), not the raw description, so the user sees exactly which future
            // transactions this standing rule will match.
            const normalizedPayee = await normalizePayee(entry.description);
            const shouldCreate = await confirmDialog(
                `Always post payees like "${normalizedPayee}" to ${trimmed}?`,
                {
                    title: 'Create standing rule?',
                    kind: 'info',
                    okLabel: 'Always',
                    cancelLabel: 'Cancel',
                },
            );
            if (!shouldCreate) {
                return;
            }
            await createResolution(ledgerPath, {
                kind: 'category-rule',
                subjectRefs: [loginScopeRef(loginName, label)],
                parts: [
                    {
                        account: trimmed,
                        amount: null,
                        ref: null,
                        notes: null,
                    },
                ],
                notes: 'Created from Pipeline "Always" action',
                predicate: {
                    descriptionRegex: null,
                    normalizedPayee,
                    amountMin: null,
                    amountMax: null,
                },
            });
            await refreshPipelineLoginAccountData();
            setPipelineStatus(
                `Always posting "${normalizedPayee}" to ${trimmed}.`,
            );
        } catch (error) {
            setPipelineStatus(`Save rule failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
        }
    }

    async function handleCreateIgnoreSourceResolution(
        entry: AccountJournalEntry,
    ) {
        if (!selectedLoginAccount) return;
        const { loginName, label } = selectedLoginAccount;
        setBusyPostEntryId(entry.id);
        try {
            await createResolution(ledgerPath, {
                kind: 'ignore-source',
                subjectRefs: [loginEntryRef(loginName, label, entry.id)],
                parts: [],
                notes: 'Created from Pipeline account row',
            });
            await refreshPipelineLoginAccountData();
            setPipelineStatus(`Ignored automation for ${entry.id}.`);
        } catch (error) {
            setPipelineStatus(`Ignore automation failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
        }
    }

    async function handlePipelinePostEntry(entryId: string) {
        setBusyPostEntryId(entryId);
        try {
            const message = await doPipelinePost(entryId);
            await refreshPipelineLoginAccountData();
            setPipelineStatus(message);
            void refreshPipelineBulkStats();
        } catch (error) {
            setPipelineStatus(`Post failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
        }
    }

    // Run the automation policy loop once after a batch post (NOT inside the
    // per-entry loop). Applies rule-backed recategorizations of pre-existing
    // Unknown GL rows + safe anomaly retires; ML suggestions stay Review. Returns
    // the number of applied proposals. Mirrors App.tsx auto-ETL phase 3.
    async function runPipelineAutomationPolicy(
        scope: AutomationScope,
    ): Promise<number> {
        try {
            const applied = await applyAutomationPolicy(ledgerPath, scope);
            if (applied.length > 0) {
                setPipelineStatus(
                    `Automation applied ${applied.length} proposal(s).`,
                );
            }
            return applied.length;
        } catch (error) {
            setPipelineStatus(`Automation policy failed: ${String(error)}`);
            return 0;
        }
    }

    async function handlePipelinePostAll() {
        if (!selectedLoginAccount) return;
        setIsPipelinePosting(true);
        try {
            for (const entry of unpostedEntries) {
                setBusyPostEntryId(entry.id);
                try {
                    const message = await doPipelinePost(entry.id);
                    await refreshPipelineLoginAccountData();
                    setPipelineStatus(message);
                } catch (error) {
                    setPipelineStatus(`Post failed: ${String(error)}`);
                    break; // stop-on-first-error
                } finally {
                    setBusyPostEntryId(null);
                }
            }
            const applied = await runPipelineAutomationPolicy({
                loginName: selectedLoginAccount.loginName,
                label: selectedLoginAccount.label,
                includeGl: true,
            });
            if (applied > 0) await refreshPipelineLoginAccountData();
        } finally {
            setIsPipelinePosting(false);
            void refreshPipelineBulkStats();
        }
    }

    async function handlePipelinePostSelected() {
        if (!selectedLoginAccount) return;
        setIsPipelinePosting(true);
        try {
            for (const entry of unpostedEntries) {
                if (!pipelineSelectedEntryIds.has(entry.id)) continue;
                setBusyPostEntryId(entry.id);
                try {
                    const message = await doPipelinePost(entry.id);
                    await refreshPipelineLoginAccountData();
                    setPipelineStatus(message);
                } catch (error) {
                    setPipelineStatus(`Post failed: ${String(error)}`);
                    break; // stop-on-first-error
                } finally {
                    setBusyPostEntryId(null);
                }
            }
            const applied = await runPipelineAutomationPolicy({
                loginName: selectedLoginAccount.loginName,
                label: selectedLoginAccount.label,
                includeGl: true,
            });
            if (applied > 0) await refreshPipelineLoginAccountData();
        } finally {
            setIsPipelinePosting(false);
            setPipelineSelectedEntryIds(new Set());
            void refreshPipelineBulkStats();
        }
    }

    async function handlePipelineExtractAllLedger() {
        setIsPipelineExtractingAllLedger(true);
        try {
            const stats = await refreshPipelineBulkStats();
            const candidates = stats.accounts.filter(
                (account) =>
                    account.extract.eligible && !account.extract.locked,
            );
            if (candidates.length === 0) {
                setPipelineStatus(
                    'No unlocked accounts are ready for extraction.',
                );
                return;
            }

            let succeeded = 0;
            let failed = 0;
            let locked = 0;
            let totalNew = 0;
            let refreshSelected = false;
            const selectedKey =
                selectedLoginAccount === null
                    ? null
                    : `${selectedLoginAccount.loginName}/${selectedLoginAccount.label}`;

            for (const [index, account] of candidates.entries()) {
                const accountKey = `${account.loginName}/${account.label}`;
                setPipelineStatus(
                    `Extract All: ${accountKey} (${index + 1}/${candidates.length})`,
                );
                try {
                    const docs = await listLoginAccountDocuments(
                        ledgerPath,
                        account.loginName,
                        account.label,
                    );
                    if (docs.length === 0) {
                        continue;
                    }
                    const result = await runLoginAccountExtraction(
                        ledgerPath,
                        account.loginName,
                        account.label,
                        docs.map((doc) => doc.filename),
                    );
                    totalNew += result.newEntryCount;
                    succeeded += 1;
                    if (selectedKey === accountKey) {
                        refreshSelected = true;
                    }
                } catch (error) {
                    if (String(error).includes('currently in use')) {
                        locked += 1;
                    } else {
                        failed += 1;
                    }
                }
            }

            if (
                refreshSelected &&
                selectedLoginAccount !== null &&
                selectedKey ===
                    `${selectedLoginAccount.loginName}/${selectedLoginAccount.label}`
            ) {
                await refreshPipelineLoginAccountData();
            }
            await refreshPipelineBulkStats();
            setPipelineStatus(
                `Extract All complete. ${succeeded} account(s) extracted, ${failed} failed, ${locked} locked, ${totalNew} new entr${totalNew === 1 ? 'y' : 'ies'} added.`,
            );
        } finally {
            setIsPipelineExtractingAllLedger(false);
        }
    }

    async function handlePipelinePostAllLedger() {
        setIsPipelinePostingAllLedger(true);
        try {
            const stats = await refreshPipelineBulkStats();
            if (stats.gl.locked) {
                setPipelineStatus('General journal is currently locked.');
                return;
            }
            const candidates = stats.accounts.filter(
                (account) => account.post.eligible && !account.post.locked,
            );
            if (candidates.length === 0) {
                setPipelineStatus(
                    'No unlocked accounts are ready for posting.',
                );
                return;
            }

            let postedCount = 0;
            let failed = 0;
            let locked = 0;
            let refreshSelected = false;
            let reloadLedgerAfter = false;
            const selectedKey =
                selectedLoginAccount === null
                    ? null
                    : `${selectedLoginAccount.loginName}/${selectedLoginAccount.label}`;

            for (const [index, account] of candidates.entries()) {
                const accountKey = `${account.loginName}/${account.label}`;
                setPipelineStatus(
                    `Post All: ${accountKey} (${index + 1}/${candidates.length})`,
                );
                try {
                    const [unposted, suggestions] = await Promise.all([
                        getLoginAccountUnposted(
                            ledgerPath,
                            account.loginName,
                            account.label,
                        ),
                        suggestCategories(
                            ledgerPath,
                            account.loginName,
                            account.label,
                        ),
                    ]);
                    for (const entry of unposted) {
                        try {
                            await doPipelinePostForAccount(
                                account.loginName,
                                account.label,
                                entry.id,
                                suggestions,
                            );
                            postedCount += 1;
                            reloadLedgerAfter = true;
                        } catch (error) {
                            if (String(error).includes('currently in use')) {
                                locked += 1;
                            } else {
                                failed += 1;
                            }
                        }
                    }
                    if (selectedKey === accountKey) {
                        refreshSelected = true;
                    }
                } catch (error) {
                    if (String(error).includes('currently in use')) {
                        locked += 1;
                    } else {
                        failed += 1;
                    }
                }
            }

            // Ledger-wide policy pass once after all accounts are posted.
            const appliedPolicy = await runPipelineAutomationPolicy({
                includeGl: true,
            });
            if (appliedPolicy > 0) {
                reloadLedgerAfter = true;
                refreshSelected = true;
            }

            if (reloadLedgerAfter) {
                onLedgerRefresh();
            }
            if (
                refreshSelected &&
                selectedLoginAccount !== null &&
                selectedKey ===
                    `${selectedLoginAccount.loginName}/${selectedLoginAccount.label}`
            ) {
                await refreshPipelineLoginAccountData();
            }
            await refreshPipelineBulkStats();
            setPipelineStatus(
                `Post All complete. ${postedCount} entr${postedCount === 1 ? 'y' : 'ies'} posted, ${failed} failed, ${locked} locked.`,
            );
        } finally {
            setIsPipelinePostingAllLedger(false);
        }
    }

    async function handlePipelineSyncEntry(entryId: string) {
        if (!selectedLoginAccount) return;
        const { loginName, label } = selectedLoginAccount;
        setBusyPostEntryId(entryId);
        try {
            const glId = await syncGlTransaction(
                ledgerPath,
                loginName,
                label,
                entryId,
            );
            await refreshPipelineLoginAccountData();
            setPipelineStatus(`Synced ${entryId} → ${glId}`);
            void refreshPipelineBulkStats();
        } catch (error) {
            setPipelineStatus(`Sync failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
        }
    }

    async function handleReviewAnomaly(anomalyId: string) {
        setBusyAnomalyId(anomalyId);
        try {
            await reviewImportAnomaly(ledgerPath, {
                id: anomalyId,
                notes: 'Reviewed in Pipeline',
            });
            await refreshPipelineLoginAccountData();
            setPipelineStatus('Import anomaly marked reviewed.');
        } catch (error) {
            setPipelineStatus(`Review failed: ${String(error)}`);
        } finally {
            setBusyAnomalyId(null);
        }
    }

    async function handleRetireAnomalySource(anomaly: ImportAnomaly) {
        setBusyAnomalyId(anomaly.id);
        try {
            await retireLoginAccountEntry(
                ledgerPath,
                anomaly.loginName,
                anomaly.label,
                anomaly.sourceEntryId,
                `import anomaly ${anomaly.id}: ${anomaly.kind}`,
            );
            await createResolution(ledgerPath, {
                kind: 'pending-retired',
                subjectRefs: [
                    loginEntryRef(
                        anomaly.loginName,
                        anomaly.label,
                        anomaly.sourceEntryId,
                    ),
                ],
                parts: [],
                notes: `Created from import anomaly ${anomaly.id}`,
            });
            await reviewImportAnomaly(ledgerPath, {
                id: anomaly.id,
                notes: 'Retired source entry from Pipeline',
            });
            await refreshPipelineLoginAccountData();
            setPipelineStatus(`Retired source entry ${anomaly.sourceEntryId}.`);
        } catch (error) {
            setPipelineStatus(`Retire failed: ${String(error)}`);
        } finally {
            setBusyAnomalyId(null);
        }
    }

    async function handleCreateSourceRelationshipResolution(
        anomaly: ImportAnomaly,
        kind: 'same-source' | 'not-same-source',
    ) {
        const evidence = anomaly.evidence[0]?.trim();
        if (evidence == null || evidence.length === 0) {
            setPipelineStatus('No evidence ref is available for this anomaly.');
            return;
        }
        setBusyAnomalyId(anomaly.id);
        try {
            await createResolution(ledgerPath, {
                kind,
                subjectRefs: [
                    loginEntryRef(
                        anomaly.loginName,
                        anomaly.label,
                        anomaly.sourceEntryId,
                    ),
                    evidenceRef(evidence),
                ],
                parts: [],
                notes: `Created from import anomaly ${anomaly.id}`,
            });
            await reviewImportAnomaly(ledgerPath, {
                id: anomaly.id,
                notes: `Saved ${kind} resolution from Pipeline`,
            });
            await refreshPipelineLoginAccountData();
            setPipelineStatus(
                `Saved ${kind} resolution for ${anomaly.sourceEntryId}.`,
            );
        } catch (error) {
            setPipelineStatus(`Save resolution failed: ${String(error)}`);
        } finally {
            setBusyAnomalyId(null);
        }
    }

    async function handleApplyAutomationProposal(proposal: AutomationProposal) {
        setBusyProposalId(proposal.id);
        try {
            const result = await applyAutomationProposal(
                ledgerPath,
                proposal.id,
            );
            await refreshPipelineLoginAccountData();
            setPipelineStatus(`Applied ${proposal.kind}: ${result}`);
            void refreshPipelineBulkStats();
        } catch (error) {
            setPipelineStatus(`Automation failed: ${String(error)}`);
        } finally {
            setBusyProposalId(null);
        }
    }

    async function handleSaveAutomationProposalDecision(
        proposal: AutomationProposal,
    ) {
        const resolution = resolutionInputFromProposal(proposal);
        if (resolution == null) {
            setPipelineStatus(
                `Cannot save ${proposal.kind} as a reusable decision.`,
            );
            return;
        }
        setBusyProposalId(proposal.id);
        try {
            await createResolution(ledgerPath, resolution);
            await refreshPipelineLoginAccountData();
            setPipelineStatus(`Saved ${resolution.kind} decision.`);
        } catch (error) {
            setPipelineStatus(`Save decision failed: ${String(error)}`);
        } finally {
            setBusyProposalId(null);
        }
    }

    async function handleDisableResolution(resolution: Resolution) {
        setBusyResolutionId(resolution.id);
        try {
            await disableResolution(ledgerPath, resolution.id);
            await refreshPipelineLoginAccountData();
            setPipelineStatus(`Disabled ${resolution.kind} decision.`);
        } catch (error) {
            setPipelineStatus(`Disable decision failed: ${String(error)}`);
        } finally {
            setBusyResolutionId(null);
        }
    }

    async function handleEnableResolution(resolution: Resolution) {
        setBusyResolutionId(resolution.id);
        try {
            await enableResolution(ledgerPath, resolution.id);
            await refreshPipelineLoginAccountData();
            setPipelineStatus(`Enabled ${resolution.kind} decision.`);
        } catch (error) {
            setPipelineStatus(`Enable decision failed: ${String(error)}`);
        } finally {
            setBusyResolutionId(null);
        }
    }

    async function handleOpenTransferModal(entryId: string) {
        if (!selectedLoginAccount) return;
        const { loginName, label } = selectedLoginAccount;
        setTransferModalEntryId(entryId);
        setTransferModalSearch('');
        setTransferModalFeePrompt(null);
        setIsLoadingTransferModal(true);
        try {
            const results = await getUnpostedEntriesForTransfer(
                ledgerPath,
                loginName,
                label,
                entryId,
            );
            setTransferModalResults(results);
        } catch (error) {
            setPipelineStatus(
                `Failed to load transfer candidates: ${String(error)}`,
            );
            setTransferModalEntryId(null);
        } finally {
            setIsLoadingTransferModal(false);
        }
    }

    /**
     * The fee residual `-(a1+a2)` between the modal's subject entry and a
     * candidate, or null when the amounts cancel / are unparseable. Amounts are
     * plain quantities (no commodity); the backend guards commodity mismatches.
     */
    function transferModalResidual(
        candidate: UnpostedTransferResult,
    ): string | null {
        const subject = accountJournalEntries.find(
            (e) => e.id === transferModalEntryId,
        );
        const a = Number(subject?.amount ?? NaN);
        const b = Number(candidate.entry.amount ?? NaN);
        if (!Number.isFinite(a) || !Number.isFinite(b)) return null;
        const residual = -(a + b);
        if (Math.abs(residual) < TRANSFER_CANCEL_EPSILON) return null;
        return residual.toFixed(2);
    }

    async function handleLinkTransferFromModal(
        other: UnpostedTransferResult,
        feeAccount?: string,
    ) {
        if (!selectedLoginAccount || transferModalEntryId === null) return;
        // Guard against a double-click firing a second concurrent post (matching
        // the per-row Link buttons, which disable on busyPostEntryId): the second
        // post races the first and fails in the backend.
        if (busyPostEntryId !== null) return;
        const { loginName, label } = selectedLoginAccount;
        setBusyPostEntryId(transferModalEntryId);
        setTransferModalEntryId(null);
        setTransferModalFeePrompt(null);
        try {
            await createTransferLinkResolution(
                ledgerPath,
                {
                    loginName,
                    label,
                    entryId: transferModalEntryId,
                },
                {
                    loginName: other.loginName,
                    label: other.label,
                    entryId: other.entry.id,
                },
            );
            const glId = await postLoginAccountTransfer(
                ledgerPath,
                loginName,
                label,
                transferModalEntryId,
                other.loginName,
                other.label,
                other.entry.id,
                feeAccount,
            );
            await refreshPipelineLoginAccountData();
            setPipelineStatus(
                `Transfer posted: ${transferModalEntryId} ↔ ${other.entry.id} (${glId})`,
            );
        } catch (error) {
            setPipelineStatus(`Transfer post failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
        }
    }

    /**
     * "Not a transfer" for an entry whose suggestion has a transferMatch:
     * record NotTransferLink negative memory for the pair (the matchers stop
     * proposing it; see automation::TransferPolicy) and refresh suggestions.
     */
    async function handleNotATransfer(
        entryId: string,
        transferMatch: { accountLocator: string; entryId: string },
    ) {
        if (!selectedLoginAccount) return;
        const { loginName, label } = selectedLoginAccount;
        const parts = transferMatch.accountLocator.split('/');
        const otherLogin = parts[1] ?? '';
        const otherLabel = parts[3] ?? '';
        if (!otherLogin || !otherLabel) {
            setPipelineStatus(
                `Not-a-transfer: could not parse locator: ${transferMatch.accountLocator}`,
            );
            return;
        }
        setBusyPostEntryId(entryId);
        try {
            await createResolution(ledgerPath, {
                kind: 'not-transfer-link',
                subjectRefs: [
                    loginEntryRef(loginName, label, entryId),
                    loginEntryRef(
                        otherLogin,
                        otherLabel,
                        transferMatch.entryId,
                    ),
                ],
                parts: [],
                notes: 'Marked not-a-transfer from Pipeline',
            });
            await refreshPipelineLoginAccountData();
            setPipelineStatus(`Marked ${entryId} as not a transfer.`);
        } catch (error) {
            setPipelineStatus(`Not-a-transfer failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
        }
    }

    // --- JSX ---

    return (
        <div className="transactions-panel">
            <section className="txn-form">
                <div className="txn-form-header">
                    <div>
                        <h2>Pipeline</h2>
                        <p>
                            Inspect the ETL stages for a bank account: evidence
                            documents, raw rows, extracted account entries, and
                            posted GL transactions.
                        </p>
                    </div>
                </div>
                <div className="txn-grid">
                    <label className="field">
                        <span>Bank account</span>
                        <select
                            value={
                                selectedLoginAccount !== null
                                    ? `${selectedLoginAccount.loginName}/${selectedLoginAccount.label}`
                                    : ''
                            }
                            disabled={
                                isPipelineExtractingAllLedger ||
                                isPipelinePostingAllLedger
                            }
                            onChange={(event) => {
                                const value = event.target.value;
                                if (!value) {
                                    setSelectedLoginAccount(null);
                                    return;
                                }
                                const [loginName, label] = value.split('/', 2);
                                if (
                                    loginName !== undefined &&
                                    label !== undefined &&
                                    loginName.length > 0 &&
                                    label.length > 0
                                ) {
                                    setSelectedLoginAccount({
                                        loginName,
                                        label,
                                    });
                                }
                            }}
                        >
                            <option value="">Select a source...</option>
                            {loginAccounts.map((account) => {
                                const key = `${account.loginName}/${account.label}`;
                                return (
                                    <option key={key} value={key}>
                                        {key}
                                    </option>
                                );
                            })}
                        </select>
                    </label>
                </div>
                <div className="pipeline-panel">
                    <div className="pipeline-actions">
                        <button
                            type="button"
                            className="primary-button"
                            disabled={
                                isLoadingPipelineBulkStats ||
                                isPipelineExtractingAllLedger ||
                                (pipelineBulkStats?.extract.eligibleAccounts ??
                                    0) === 0
                            }
                            onClick={() => {
                                void handlePipelineExtractAllLedger();
                            }}
                        >
                            {isPipelineExtractingAllLedger
                                ? 'Extracting...'
                                : `Extract All (${pipelineBulkStats?.extract.totalDocuments ?? 0})`}
                        </button>
                        <button
                            type="button"
                            className="primary-button"
                            disabled={
                                isLoadingPipelineBulkStats ||
                                isPipelinePostingAllLedger ||
                                glLockStatus.locked ||
                                (pipelineBulkStats?.post.eligibleAccounts ??
                                    0) === 0
                            }
                            onClick={() => {
                                void handlePipelinePostAllLedger();
                            }}
                        >
                            {isPipelinePostingAllLedger
                                ? 'Posting...'
                                : `Post All (${pipelineBulkStats?.post.totalUnpostedEntries ?? 0})`}
                        </button>
                        <button
                            type="button"
                            className="ghost-button"
                            disabled={isLoadingPipelineBulkStats}
                            onClick={() => {
                                void refreshPipelineBulkStats();
                            }}
                        >
                            {isLoadingPipelineBulkStats
                                ? 'Refreshing stats...'
                                : 'Refresh Stats'}
                        </button>
                    </div>
                    {pipelineStatus !== null && (
                        <p className="status">{pipelineStatus}</p>
                    )}
                    {pipelineBulkStats !== null && (
                        <div className="hint">
                            <div>
                                {`Extract All: ${pipelineBulkStats.extract.totalDocuments} document(s) across ${pipelineBulkStats.extract.eligibleAccounts} eligible account(s).`}
                            </div>
                            <div>
                                {`${pipelineBulkStats.extract.skippedMissingExtension} missing extension, ${pipelineBulkStats.extract.skippedMissingExtractor} missing extractor, ${pipelineBulkStats.extract.skippedNoDocuments} no documents, ${pipelineBulkStats.extract.inspectFailures} inspect failures, ${pipelineBulkStats.extract.lockedAccounts} locked.`}
                            </div>
                            <div>
                                {`Post All: ${pipelineBulkStats.post.totalUnpostedEntries} entr${pipelineBulkStats.post.totalUnpostedEntries === 1 ? 'y' : 'ies'} across ${pipelineBulkStats.post.eligibleAccounts} eligible account(s).`}
                            </div>
                            <div>
                                {`${pipelineBulkStats.post.skippedMissingGlAccount} missing GL mapping, ${pipelineBulkStats.post.skippedNoUnposted} no unposted, ${pipelineBulkStats.post.inspectFailures} inspect failures, ${pipelineBulkStats.post.lockedAccounts} locked, GL ${pipelineBulkStats.gl.locked ? 'locked' : 'unlocked'}.`}
                            </div>
                        </div>
                    )}
                    {pipelineBulkStats !== null && (
                        <table className="pipeline-status-table">
                            <tbody>
                                {pipelineBulkStats.accounts.map((acct, i) => {
                                    const prevLogin =
                                        i > 0
                                            ? pipelineBulkStats.accounts[i - 1]
                                                  ?.loginName
                                            : null;
                                    const isSelected =
                                        selectedLoginAccount?.loginName ===
                                            acct.loginName &&
                                        selectedLoginAccount.label ===
                                            acct.label;
                                    const extractChip =
                                        acct.extract.inspectError !== null ? (
                                            <span className="status-chip status-chip-warning">
                                                {acct.extract.inspectError.slice(
                                                    0,
                                                    40,
                                                )}
                                            </span>
                                        ) : acct.extract.skipReason ===
                                          'missing-extension' ? (
                                            <span className="status-chip status-chip-warning">
                                                no extension
                                            </span>
                                        ) : acct.extract.skipReason ===
                                              'missing-extractor' ||
                                          acct.extract.skipReason ===
                                              'broken-extractor' ? (
                                            <span className="status-chip status-chip-warning">
                                                {acct.extract.skipReason}
                                            </span>
                                        ) : acct.extract.skipReason ===
                                          'no-documents' ? (
                                            <span className="status-chip">
                                                no docs
                                            </span>
                                        ) : (
                                            <span className="status-chip status-chip-ok">
                                                {`${acct.extract.documentCount} doc${acct.extract.documentCount === 1 ? '' : 's'}`}
                                                {acct.extract.locked
                                                    ? ' 🔒'
                                                    : ''}
                                            </span>
                                        );
                                    const postChip =
                                        acct.post.inspectError !== null ? (
                                            <span className="status-chip status-chip-warning">
                                                {acct.post.inspectError.slice(
                                                    0,
                                                    40,
                                                )}
                                            </span>
                                        ) : acct.post.skipReason ===
                                          'missing-gl-account' ? (
                                            <span className="status-chip status-chip-warning">
                                                no GL account
                                            </span>
                                        ) : acct.post.skipReason ===
                                          'no-unposted' ? (
                                            <span className="status-chip">
                                                up to date
                                            </span>
                                        ) : (
                                            <span className="status-chip status-chip-ok">
                                                {`${acct.post.unpostedCount} unposted`}
                                                {acct.post.locked ? ' 🔒' : ''}
                                            </span>
                                        );
                                    return (
                                        <Fragment
                                            key={`${acct.loginName}/${acct.label}`}
                                        >
                                            {acct.loginName !== prevLogin && (
                                                <tr className="login-group-header">
                                                    <td colSpan={3}>
                                                        {acct.loginName}
                                                    </td>
                                                </tr>
                                            )}
                                            <tr
                                                className={`account-row${isSelected ? ' selected' : ''}`}
                                                onClick={() => {
                                                    setSelectedLoginAccount({
                                                        loginName:
                                                            acct.loginName,
                                                        label: acct.label,
                                                    });
                                                }}
                                            >
                                                <td>{acct.label}</td>
                                                <td>{extractChip}</td>
                                                <td>{postChip}</td>
                                            </tr>
                                        </Fragment>
                                    );
                                })}
                            </tbody>
                        </table>
                    )}
                    {selectedLoginLockStatus?.locked === true && (
                        <p className="status">
                            {selectedLoginLockStatus.metadata === null
                                ? 'Selected login is currently in use by another operation.'
                                : `Selected login locked by ${selectedLoginLockStatus.metadata.owner}/${selectedLoginLockStatus.metadata.purpose}.`}
                        </p>
                    )}
                    {glLockStatus.locked && (
                        <p className="status">
                            {glLockStatus.metadata === null
                                ? 'General journal is currently in use by another operation.'
                                : `General journal locked by ${glLockStatus.metadata.owner}/${glLockStatus.metadata.purpose}.`}
                        </p>
                    )}
                </div>
                <div className="tabs pipeline-subtabs">
                    <button
                        className={
                            pipelineSubTab === 'evidence' ? 'tab active' : 'tab'
                        }
                        onClick={() => {
                            setPipelineSubTab('evidence');
                        }}
                        type="button"
                    >
                        Evidence
                    </button>
                    <button
                        className={
                            pipelineSubTab === 'evidence-rows'
                                ? 'tab active'
                                : 'tab'
                        }
                        onClick={() => {
                            setPipelineSubTab('evidence-rows');
                        }}
                        type="button"
                    >
                        Evidence Rows
                    </button>
                    <button
                        className={
                            pipelineSubTab === 'account-rows'
                                ? 'tab active'
                                : 'tab'
                        }
                        onClick={() => {
                            setPipelineSubTab('account-rows');
                        }}
                        type="button"
                    >
                        Account Rows
                    </button>
                </div>
                {selectedLoginAccount === null ? (
                    <p className="hint">
                        Select a bank account to inspect its pipeline stages.
                    </p>
                ) : pipelineSubTab === 'evidence' ? (
                    isLoadingDocuments ? (
                        <p className="status">Loading documents...</p>
                    ) : documents.length === 0 ? (
                        <p className="hint">
                            No documents found for this account.
                        </p>
                    ) : (
                        <div className="table-wrap">
                            <table className="ledger-table">
                                <thead>
                                    <tr>
                                        <th>Document</th>
                                        <th>Type</th>
                                        <th>Coverage End</th>
                                        <th>Scraped At</th>
                                        <th>Scrape Session</th>
                                        <th></th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {documents.map((doc) => (
                                        <tr key={doc.filename}>
                                            <td className="mono">
                                                {doc.filename}
                                            </td>
                                            <td className="mono">
                                                {doc.info?.mimeType ?? '-'}
                                            </td>
                                            <td className="mono">
                                                {doc.info?.coverageEndDate ??
                                                    '-'}
                                            </td>
                                            <td className="mono">
                                                {doc.info?.scrapedAt ?? '-'}
                                            </td>
                                            <td className="mono">
                                                {doc.info?.scrapeSessionId ??
                                                    '-'}
                                            </td>
                                            <td>
                                                {!isPdfDocument(
                                                    doc.filename,
                                                ) && (
                                                    <button
                                                        type="button"
                                                        onClick={() => {
                                                            void handleDocumentView(
                                                                doc,
                                                            );
                                                        }}
                                                    >
                                                        View
                                                    </button>
                                                )}
                                            </td>
                                        </tr>
                                    ))}
                                </tbody>
                            </table>
                        </div>
                    )
                ) : pipelineSubTab === 'evidence-rows' ? (
                    <>
                        <div className="txn-grid">
                            <label className="field">
                                <span>Document</span>
                                <select
                                    value={evidenceRowsDocument}
                                    onChange={(e) => {
                                        void handleLoadDocumentRows(
                                            e.target.value,
                                        );
                                    }}
                                >
                                    <option value="">
                                        Select a document...
                                    </option>
                                    {documents
                                        .filter((d) =>
                                            d.filename
                                                .toLowerCase()
                                                .endsWith('.csv'),
                                        )
                                        .map((d) => (
                                            <option
                                                key={d.filename}
                                                value={d.filename}
                                            >
                                                {d.filename}
                                            </option>
                                        ))}
                                </select>
                            </label>
                            <div className="field">
                                <span />
                                <button
                                    type="button"
                                    className="primary-button"
                                    onClick={() => {
                                        void handlePipelineExtraction(
                                            evidenceRowsDocument,
                                        );
                                    }}
                                    disabled={
                                        !evidenceRowsDocument ||
                                        isRunningExtraction ||
                                        !hasExtension ||
                                        selectedLoginLocked ||
                                        isPipelineExtractingAllLedger
                                    }
                                >
                                    {isRunningExtraction
                                        ? 'Extracting...'
                                        : evidencedRowNumbers.size > 0
                                          ? 'Re-extract document → account rows'
                                          : 'Extract document → account rows'}
                                </button>
                                {!hasExtension && (
                                    <span className="hint">
                                        No extension bound — set one in
                                        Scraping.
                                    </span>
                                )}
                            </div>
                        </div>
                        {evidenceRowsDocument === '' ? (
                            <p className="hint">
                                Select a CSV document to view its raw rows.
                            </p>
                        ) : isLoadingDocumentRows ? (
                            <p className="status">Loading rows...</p>
                        ) : documentRows.length === 0 ? (
                            <p className="hint">No rows found.</p>
                        ) : (
                            <div className="table-wrap">
                                <table className="ledger-table">
                                    <tbody>
                                        {documentRows.map((row, rowIndex) => {
                                            const rowNum = rowIndex + 1;
                                            const isHeader = rowIndex === 0;
                                            const isEvidenced =
                                                evidencedRowNumbers.has(rowNum);
                                            return (
                                                <tr
                                                    key={rowIndex}
                                                    className={
                                                        isHeader
                                                            ? 'evidence-header-row'
                                                            : isEvidenced
                                                              ? 'evidence-highlighted-row'
                                                              : undefined
                                                    }
                                                >
                                                    <td className="mono">
                                                        {rowNum}
                                                    </td>
                                                    {row.map(
                                                        (cell, colIndex) => (
                                                            <td
                                                                key={colIndex}
                                                                className="mono"
                                                            >
                                                                {cell}
                                                            </td>
                                                        ),
                                                    )}
                                                </tr>
                                            );
                                        })}
                                    </tbody>
                                </table>
                            </div>
                        )}
                    </>
                ) : isLoadingAccountJournal ? (
                    <p className="status">Loading account rows...</p>
                ) : (
                    <div className="account-rows-panel">
                        <div className="pipeline-panel">
                            <div className="pipeline-actions pipeline-gl-account-row">
                                <span className="pipeline-gl-account-label">
                                    GL Account:
                                </span>
                                {pipelineGlAccount !== null &&
                                pipelineGlAccount !== '' ? (
                                    <span className="mono">
                                        {pipelineGlAccount}
                                    </span>
                                ) : (
                                    <>
                                        <input
                                            type="text"
                                            placeholder={suggestGlAccountName(
                                                selectedLoginAccount.label,
                                            )}
                                            value={pipelineGlAccountDraft}
                                            onChange={(e) => {
                                                setPipelineGlAccountDraft(
                                                    e.target.value,
                                                );
                                            }}
                                        />
                                        <button
                                            type="button"
                                            className="primary-button"
                                            disabled={
                                                isSavingPipelineGlAccount ||
                                                selectedLoginLocked ||
                                                pipelineGlAccountDraft.trim()
                                                    .length === 0
                                            }
                                            onClick={() => {
                                                void handleSavePipelineGlAccount();
                                            }}
                                        >
                                            {isSavingPipelineGlAccount
                                                ? 'Saving...'
                                                : 'Save GL Account'}
                                        </button>
                                    </>
                                )}
                            </div>
                            <div className="pipeline-actions">
                                <button
                                    type="button"
                                    className="primary-button"
                                    disabled={
                                        isPipelinePosting ||
                                        isPipelinePostingAllLedger ||
                                        glLockStatus.locked ||
                                        selectedLoginLocked ||
                                        unpostedEntries.length === 0
                                    }
                                    onClick={() => {
                                        void handlePipelinePostAll();
                                    }}
                                >
                                    {isPipelinePosting
                                        ? 'Posting...'
                                        : `Post All (${unpostedEntries.length.toString()})`}
                                </button>
                                <button
                                    type="button"
                                    className="ghost-button"
                                    disabled={
                                        isPipelinePosting ||
                                        isPipelinePostingAllLedger ||
                                        glLockStatus.locked ||
                                        selectedLoginLocked ||
                                        pipelineSelectedEntryIds.size === 0
                                    }
                                    onClick={() => {
                                        void handlePipelinePostSelected();
                                    }}
                                >
                                    {`Post Selected (${pipelineSelectedEntryIds.size.toString()})`}
                                </button>
                            </div>
                        </div>
                        {automationProposals.length > 0 && (
                            <div className="pipeline-panel">
                                <h3>Automation proposals</h3>
                                <div className="table-wrap">
                                    <table className="ledger-table">
                                        <thead>
                                            <tr>
                                                <th>Kind</th>
                                                <th>Policy</th>
                                                <th>Applies to</th>
                                                <th>Result</th>
                                                <th>Undo</th>
                                                <th>Reasons</th>
                                                <th>Actions</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {automationProposals.map(
                                                (proposal) => {
                                                    const result =
                                                        proposal.proposedResult
                                                            .suggestedAccount ??
                                                        proposal.proposedResult
                                                            .transferMatch
                                                            ?.entryId ??
                                                        (proposal.proposedResult
                                                            .parts.length > 0
                                                            ? proposal.proposedResult.parts
                                                                  .map((part) =>
                                                                      `${part.account ?? '(account)'} ${part.amount ?? ''}`.trim(),
                                                                  )
                                                                  .join(' + ')
                                                            : null) ??
                                                        proposal.proposedResult
                                                            .notes ??
                                                        proposal.proposedResult
                                                            .importAnomalyId ??
                                                        '-';
                                                    const reasons =
                                                        proposal.reasons
                                                            .map(
                                                                (reason) =>
                                                                    `${reason.field}: ${reason.detail}`,
                                                            )
                                                            .join(', ');
                                                    const isApplyable =
                                                        proposal.blockers
                                                            .length === 0 &&
                                                        proposal.canApply;
                                                    const canSaveDecision =
                                                        proposal.blockers
                                                            .length === 0 &&
                                                        resolutionInputFromProposal(
                                                            proposal,
                                                        ) != null;
                                                    return (
                                                        <tr key={proposal.id}>
                                                            <td>
                                                                {proposal.kind}
                                                            </td>
                                                            <td>
                                                                {
                                                                    proposal.policyDecision
                                                                }
                                                            </td>
                                                            <td>
                                                                {proposal.subjectRefs
                                                                    .map(
                                                                        refLabel,
                                                                    )
                                                                    .join(
                                                                        ' <-> ',
                                                                    )}
                                                            </td>
                                                            <td>{result}</td>
                                                            <td>
                                                                {
                                                                    proposal.reversible
                                                                }
                                                            </td>
                                                            <td>
                                                                {proposal
                                                                    .blockers
                                                                    .length ===
                                                                0
                                                                    ? reasons
                                                                    : proposal.blockers
                                                                          .map(
                                                                              (
                                                                                  blocker,
                                                                              ) =>
                                                                                  blocker.detail,
                                                                          )
                                                                          .join(
                                                                              ', ',
                                                                          )}
                                                            </td>
                                                            <td>
                                                                <div className="txn-actions">
                                                                    <button
                                                                        type="button"
                                                                        className="ghost-button"
                                                                        disabled={
                                                                            !isApplyable ||
                                                                            busyProposalId ===
                                                                                proposal.id
                                                                        }
                                                                        onClick={() => {
                                                                            void handleApplyAutomationProposal(
                                                                                proposal,
                                                                            );
                                                                        }}
                                                                    >
                                                                        Apply
                                                                    </button>
                                                                    <button
                                                                        type="button"
                                                                        className="ghost-button"
                                                                        disabled={
                                                                            !canSaveDecision ||
                                                                            busyProposalId ===
                                                                                proposal.id
                                                                        }
                                                                        onClick={() => {
                                                                            void handleSaveAutomationProposalDecision(
                                                                                proposal,
                                                                            );
                                                                        }}
                                                                    >
                                                                        Save
                                                                        decision
                                                                    </button>
                                                                </div>
                                                            </td>
                                                        </tr>
                                                    );
                                                },
                                            )}
                                        </tbody>
                                    </table>
                                </div>
                            </div>
                        )}
                        {activeResolutions.length > 0 && (
                            <div className="pipeline-panel">
                                <h3>Saved decisions</h3>
                                <div className="table-wrap">
                                    <table className="ledger-table">
                                        <thead>
                                            <tr>
                                                <th>Kind</th>
                                                <th>Status</th>
                                                <th>Applies to</th>
                                                <th>Outcome</th>
                                                <th>Updated</th>
                                                <th>Notes</th>
                                                <th>Actions</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {activeResolutions.map(
                                                (resolution) => (
                                                    <tr key={resolution.id}>
                                                        <td>
                                                            {resolution.kind}
                                                        </td>
                                                        <td>
                                                            {resolution.status}
                                                        </td>
                                                        <td>
                                                            {resolutionSubjectLabel(
                                                                resolution,
                                                            )}
                                                        </td>
                                                        <td>
                                                            {resolutionResultLabel(
                                                                resolution,
                                                            )}
                                                        </td>
                                                        <td className="mono">
                                                            {
                                                                resolution.updatedAt
                                                            }
                                                        </td>
                                                        <td>
                                                            <div>
                                                                {resolution.notes ??
                                                                    '-'}
                                                            </div>
                                                            <div className="hint mono">
                                                                {resolution.id}
                                                            </div>
                                                        </td>
                                                        <td>
                                                            <div className="txn-actions">
                                                                <button
                                                                    type="button"
                                                                    className="ghost-button"
                                                                    disabled={
                                                                        resolution.status !==
                                                                            'active' ||
                                                                        busyResolutionId ===
                                                                            resolution.id
                                                                    }
                                                                    onClick={() => {
                                                                        void handleDisableResolution(
                                                                            resolution,
                                                                        );
                                                                    }}
                                                                >
                                                                    Disable
                                                                </button>
                                                                <button
                                                                    type="button"
                                                                    className="ghost-button"
                                                                    disabled={
                                                                        resolution.status !==
                                                                            'disabled' ||
                                                                        busyResolutionId ===
                                                                            resolution.id
                                                                    }
                                                                    onClick={() => {
                                                                        void handleEnableResolution(
                                                                            resolution,
                                                                        );
                                                                    }}
                                                                >
                                                                    Enable
                                                                </button>
                                                            </div>
                                                        </td>
                                                    </tr>
                                                ),
                                            )}
                                        </tbody>
                                    </table>
                                </div>
                            </div>
                        )}
                        {importAnomalies.length > 0 && (
                            <div className="pipeline-panel">
                                <h3>Import anomalies</h3>
                                <div className="table-wrap">
                                    <table className="ledger-table">
                                        <thead>
                                            <tr>
                                                <th>Kind</th>
                                                <th>Date</th>
                                                <th>Description</th>
                                                <th>Amount</th>
                                                <th>Source entry</th>
                                                <th>Evidence</th>
                                                <th>Reason</th>
                                                <th>Actions</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {importAnomalies.map((anomaly) => (
                                                <tr key={anomaly.id}>
                                                    <td>{anomaly.kind}</td>
                                                    <td className="mono">
                                                        {anomaly.date}
                                                    </td>
                                                    <td>
                                                        {anomaly.description}
                                                    </td>
                                                    <td className="mono">
                                                        {anomaly.amount ?? '-'}
                                                    </td>
                                                    <td className="mono">
                                                        {anomaly.sourceEntryId}
                                                    </td>
                                                    <td>
                                                        {anomaly.evidence
                                                            .length === 0
                                                            ? '-'
                                                            : anomaly.evidence.join(
                                                                  ', ',
                                                              )}
                                                    </td>
                                                    <td>
                                                        {anomaly.safetyReasons
                                                            .length === 0
                                                            ? anomaly.coverageDocument
                                                            : anomaly.safetyReasons.join(
                                                                  ', ',
                                                              )}
                                                    </td>
                                                    <td>
                                                        <div className="txn-actions">
                                                            {anomaly.glTxnId !=
                                                                null && (
                                                                <button
                                                                    type="button"
                                                                    className="ghost-button"
                                                                    onClick={() => {
                                                                        if (
                                                                            anomaly.glTxnId !=
                                                                            null
                                                                        )
                                                                            onViewGlTransaction(
                                                                                anomaly.glTxnId,
                                                                            );
                                                                    }}
                                                                >
                                                                    View GL
                                                                </button>
                                                            )}
                                                            <button
                                                                type="button"
                                                                className="ghost-button"
                                                                disabled={
                                                                    busyAnomalyId ===
                                                                    anomaly.id
                                                                }
                                                                onClick={() => {
                                                                    void handleReviewAnomaly(
                                                                        anomaly.id,
                                                                    );
                                                                }}
                                                            >
                                                                Dismiss
                                                            </button>
                                                            {anomaly.safeToRetire && (
                                                                <button
                                                                    type="button"
                                                                    className="ghost-button"
                                                                    disabled={
                                                                        busyAnomalyId ===
                                                                        anomaly.id
                                                                    }
                                                                    onClick={() => {
                                                                        void handleRetireAnomalySource(
                                                                            anomaly,
                                                                        );
                                                                    }}
                                                                >
                                                                    Retire
                                                                    source
                                                                </button>
                                                            )}
                                                            {anomaly.kind ===
                                                                'duplicate-import-repair-skipped' &&
                                                                anomaly.evidence
                                                                    .length >
                                                                    0 && (
                                                                    <>
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            disabled={
                                                                                busyAnomalyId ===
                                                                                anomaly.id
                                                                            }
                                                                            onClick={() => {
                                                                                void handleCreateSourceRelationshipResolution(
                                                                                    anomaly,
                                                                                    'same-source',
                                                                                );
                                                                            }}
                                                                        >
                                                                            Merge
                                                                        </button>
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            disabled={
                                                                                busyAnomalyId ===
                                                                                anomaly.id
                                                                            }
                                                                            onClick={() => {
                                                                                void handleCreateSourceRelationshipResolution(
                                                                                    anomaly,
                                                                                    'not-same-source',
                                                                                );
                                                                            }}
                                                                        >
                                                                            Keep
                                                                            separate
                                                                        </button>
                                                                    </>
                                                                )}
                                                        </div>
                                                    </td>
                                                </tr>
                                            ))}
                                        </tbody>
                                    </table>
                                </div>
                            </div>
                        )}
                        <div className="table-wrap">
                            <table className="ledger-table">
                                <thead>
                                    <tr>
                                        <th></th>
                                        <th>Date</th>
                                        <th>Description</th>
                                        <th>Amount</th>
                                        <th>Status</th>
                                        <th>Actions</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {accountJournalEntries.length === 0 ? (
                                        <tr>
                                            <td
                                                colSpan={6}
                                                className="table-empty"
                                            >
                                                No entries found.
                                            </td>
                                        </tr>
                                    ) : (
                                        accountJournalEntries.map((entry) => {
                                            const suggestion =
                                                pipelineCategorySuggestions[
                                                    entry.id
                                                ];
                                            // Local const so the narrowed
                                            // (non-null) type survives into
                                            // the "Always" onClick closure.
                                            const suggestedAccount:
                                                | string
                                                | null =
                                                suggestion?.suggested ?? null;
                                            const amountChanged =
                                                suggestion?.amountChanged ??
                                                false;
                                            const statusChanged =
                                                suggestion?.statusChanged ??
                                                false;
                                            const needsSync =
                                                entry.posted !== null &&
                                                (amountChanged ||
                                                    statusChanged);
                                            const isUnposted =
                                                entry.posted === null;
                                            const transferMatch =
                                                suggestion?.transferMatch ??
                                                null;
                                            // Near-miss transfer candidates
                                            // (2+ ambiguous matches; see
                                            // CategoryResult.transferCandidates).
                                            const transferCandidateCount =
                                                suggestion?.transferCandidates
                                                    .length ?? 0;
                                            const isBusy =
                                                busyPostEntryId === entry.id;
                                            const isSelected =
                                                pipelineSelectedEntryIds.has(
                                                    entry.id,
                                                );
                                            return (
                                                <tr
                                                    key={entry.id}
                                                    className={
                                                        needsSync
                                                            ? 'row-needs-sync'
                                                            : undefined
                                                    }
                                                >
                                                    <td>
                                                        {isUnposted && (
                                                            <input
                                                                type="checkbox"
                                                                checked={
                                                                    isSelected
                                                                }
                                                                onChange={(
                                                                    e,
                                                                ) => {
                                                                    setPipelineSelectedEntryIds(
                                                                        (
                                                                            current,
                                                                        ) => {
                                                                            const next =
                                                                                new Set(
                                                                                    current,
                                                                                );
                                                                            if (
                                                                                e
                                                                                    .target
                                                                                    .checked
                                                                            )
                                                                                next.add(
                                                                                    entry.id,
                                                                                );
                                                                            else
                                                                                next.delete(
                                                                                    entry.id,
                                                                                );
                                                                            return next;
                                                                        },
                                                                    );
                                                                }}
                                                            />
                                                        )}
                                                    </td>
                                                    <td className="mono">
                                                        {entry.date}
                                                    </td>
                                                    <td>{entry.description}</td>
                                                    <td className="mono">
                                                        {entry.amount ?? '-'}
                                                    </td>
                                                    <td>
                                                        <div className="transaction-bookkeeping-badges">
                                                            {entry.bankStatus ===
                                                            'pending' ? (
                                                                <span className="status-chip status-chip-warning">
                                                                    bank pending
                                                                </span>
                                                            ) : entry.bankStatus ===
                                                              'posted' ? (
                                                                <span className="status-chip status-chip-ok">
                                                                    bank posted
                                                                </span>
                                                            ) : null}
                                                            {isUnposted ? (
                                                                <span className="status-chip">
                                                                    unposted
                                                                </span>
                                                            ) : needsSync ? (
                                                                <span className="status-chip status-chip-warning">
                                                                    needs sync
                                                                </span>
                                                            ) : (
                                                                <span className="status-chip status-chip-ok">
                                                                    posted
                                                                </span>
                                                            )}
                                                        </div>
                                                    </td>
                                                    <td>
                                                        <div className="pipeline-row-actions">
                                                            {isUnposted ? (
                                                                <>
                                                                    <button
                                                                        type="button"
                                                                        className="primary-button"
                                                                        disabled={
                                                                            isBusy ||
                                                                            isPipelinePosting ||
                                                                            isPipelinePostingAllLedger ||
                                                                            glLockStatus.locked ||
                                                                            selectedLoginLocked
                                                                        }
                                                                        onClick={() => {
                                                                            void handlePipelinePostEntry(
                                                                                entry.id,
                                                                            );
                                                                        }}
                                                                    >
                                                                        {isBusy
                                                                            ? 'Posting...'
                                                                            : 'Post'}
                                                                    </button>
                                                                    <button
                                                                        type="button"
                                                                        className="ghost-button"
                                                                        disabled={
                                                                            isBusy ||
                                                                            isPipelinePosting ||
                                                                            isPipelinePostingAllLedger ||
                                                                            glLockStatus.locked ||
                                                                            selectedLoginLocked
                                                                        }
                                                                        onClick={() => {
                                                                            setSplitDraftRows(
                                                                                [
                                                                                    {
                                                                                        account:
                                                                                            '',
                                                                                        amount: '',
                                                                                    },
                                                                                    {
                                                                                        account:
                                                                                            '',
                                                                                        amount: '',
                                                                                    },
                                                                                ],
                                                                            );
                                                                            setSplitModalEntryId(
                                                                                entry.id,
                                                                            );
                                                                        }}
                                                                    >
                                                                        Split
                                                                    </button>
                                                                    {suggestedAccount !=
                                                                        null && (
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            disabled={
                                                                                isBusy
                                                                            }
                                                                            title={`Always post payees like "${entry.description}" to ${suggestedAccount}`}
                                                                            onClick={() => {
                                                                                void handleCreateCategoryResolution(
                                                                                    entry,
                                                                                    suggestedAccount,
                                                                                );
                                                                            }}
                                                                        >
                                                                            Always:{' '}
                                                                            {
                                                                                suggestedAccount
                                                                            }
                                                                        </button>
                                                                    )}
                                                                    {(entry.isTransfer ||
                                                                        transferMatch !==
                                                                            null ||
                                                                        transferCandidateCount >=
                                                                            2) && (
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            disabled={
                                                                                isBusy ||
                                                                                isPipelinePosting ||
                                                                                isPipelinePostingAllLedger ||
                                                                                glLockStatus.locked ||
                                                                                selectedLoginLocked
                                                                            }
                                                                            onClick={() => {
                                                                                void handleOpenTransferModal(
                                                                                    entry.id,
                                                                                );
                                                                            }}
                                                                        >
                                                                            {transferCandidateCount >=
                                                                            2
                                                                                ? `Link Transfer (${transferCandidateCount} possible)`
                                                                                : 'Link Transfer'}
                                                                        </button>
                                                                    )}
                                                                    {transferMatch !==
                                                                        null && (
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            disabled={
                                                                                isBusy
                                                                            }
                                                                            title="Remember this pair is NOT a transfer (stops auto-matching)"
                                                                            onClick={() => {
                                                                                void handleNotATransfer(
                                                                                    entry.id,
                                                                                    transferMatch,
                                                                                );
                                                                            }}
                                                                        >
                                                                            Not
                                                                            a
                                                                            transfer
                                                                        </button>
                                                                    )}
                                                                    <button
                                                                        type="button"
                                                                        className="ghost-button"
                                                                        disabled={
                                                                            isBusy
                                                                        }
                                                                        onClick={() => {
                                                                            void handleCreateIgnoreSourceResolution(
                                                                                entry,
                                                                            );
                                                                        }}
                                                                    >
                                                                        Ignore
                                                                        automation
                                                                    </button>
                                                                </>
                                                            ) : (
                                                                <>
                                                                    {needsSync && (
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            disabled={
                                                                                isBusy ||
                                                                                isPipelinePosting ||
                                                                                isPipelinePostingAllLedger ||
                                                                                glLockStatus.locked ||
                                                                                selectedLoginLocked
                                                                            }
                                                                            onClick={() => {
                                                                                void handlePipelineSyncEntry(
                                                                                    entry.id,
                                                                                );
                                                                            }}
                                                                        >
                                                                            {isBusy
                                                                                ? 'Syncing...'
                                                                                : 'Sync'}
                                                                        </button>
                                                                    )}
                                                                    <button
                                                                        type="button"
                                                                        className="ghost-button"
                                                                        onClick={() => {
                                                                            const parts =
                                                                                (
                                                                                    entry.posted ??
                                                                                    ''
                                                                                ).split(
                                                                                    ':',
                                                                                );
                                                                            const glTxnId =
                                                                                parts[
                                                                                    parts.length -
                                                                                        1
                                                                                ] ??
                                                                                '';
                                                                            if (
                                                                                glTxnId
                                                                            ) {
                                                                                onViewGlTransaction(
                                                                                    glTxnId,
                                                                                );
                                                                            }
                                                                        }}
                                                                    >
                                                                        View
                                                                    </button>
                                                                </>
                                                            )}
                                                        </div>
                                                    </td>
                                                </tr>
                                            );
                                        })
                                    )}
                                </tbody>
                            </table>
                        </div>
                        {transferModalEntryId !== null && (
                            <div
                                className="modal-overlay"
                                onClick={() => {
                                    setTransferModalEntryId(null);
                                }}
                            >
                                <div
                                    className="modal-dialog"
                                    onClick={(e) => {
                                        e.stopPropagation();
                                    }}
                                >
                                    <div className="modal-header">
                                        <h3>Link Transfer</h3>
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                setTransferModalEntryId(null);
                                            }}
                                        >
                                            Close
                                        </button>
                                    </div>
                                    <input
                                        type="search"
                                        placeholder="Search candidates…"
                                        value={transferModalSearch}
                                        onChange={(e) => {
                                            setTransferModalSearch(
                                                e.target.value,
                                            );
                                        }}
                                    />
                                    {transferModalFeePrompt !== null && (
                                        <div className="status">
                                            <div>
                                                The selected entry does not
                                                cancel this amount; the
                                                difference of{' '}
                                                {
                                                    transferModalFeePrompt.residual
                                                }{' '}
                                                will post to the fee account
                                                below.
                                            </div>
                                            <AccountInput
                                                value={transferModalFeeAccount}
                                                onChange={
                                                    setTransferModalFeeAccount
                                                }
                                                accounts={ledger.accounts.map(
                                                    (a) => a.name,
                                                )}
                                                placeholder="Fee account…"
                                            />
                                            {(transferModalFeeAccount
                                                .trim()
                                                .startsWith('Assets:') ||
                                                transferModalFeeAccount
                                                    .trim()
                                                    .startsWith(
                                                        'Liabilities:',
                                                    )) && (
                                                <div className="hint">
                                                    Fee must be an
                                                    expense/income account, not
                                                    a balance-sheet account.
                                                </div>
                                            )}
                                            <button
                                                type="button"
                                                className="ghost-button"
                                                disabled={
                                                    transferModalFeeAccount.trim() ===
                                                        '' ||
                                                    transferModalFeeAccount
                                                        .trim()
                                                        .startsWith(
                                                            'Assets:',
                                                        ) ||
                                                    transferModalFeeAccount
                                                        .trim()
                                                        .startsWith(
                                                            'Liabilities:',
                                                        ) ||
                                                    busyPostEntryId !== null
                                                }
                                                onClick={() => {
                                                    void handleLinkTransferFromModal(
                                                        transferModalFeePrompt.candidate,
                                                        transferModalFeeAccount.trim(),
                                                    );
                                                }}
                                            >
                                                Link with fee
                                            </button>
                                            <button
                                                type="button"
                                                className="ghost-button"
                                                onClick={() => {
                                                    setTransferModalFeePrompt(
                                                        null,
                                                    );
                                                }}
                                            >
                                                Cancel
                                            </button>
                                        </div>
                                    )}
                                    {isLoadingTransferModal ? (
                                        <p className="status">
                                            Loading entries...
                                        </p>
                                    ) : (
                                        <div className="table-wrap">
                                            <table className="ledger-table">
                                                <thead>
                                                    <tr>
                                                        <th>Date</th>
                                                        <th>Login/Label</th>
                                                        <th>Description</th>
                                                        <th>Amount</th>
                                                        <th></th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {visibleTransferResults.length ===
                                                    0 ? (
                                                        <tr>
                                                            <td
                                                                colSpan={5}
                                                                className="table-empty"
                                                            >
                                                                No unposted
                                                                entries found in
                                                                other accounts.
                                                            </td>
                                                        </tr>
                                                    ) : (
                                                        visibleTransferResults.map(
                                                            (r) => (
                                                                <tr
                                                                    key={
                                                                        r.entry
                                                                            .id
                                                                    }
                                                                >
                                                                    <td className="mono">
                                                                        {
                                                                            r
                                                                                .entry
                                                                                .date
                                                                        }
                                                                    </td>
                                                                    <td>
                                                                        {
                                                                            r.loginName
                                                                        }
                                                                        /
                                                                        {
                                                                            r.label
                                                                        }
                                                                    </td>
                                                                    <td>
                                                                        {
                                                                            r
                                                                                .entry
                                                                                .description
                                                                        }
                                                                    </td>
                                                                    <td className="mono">
                                                                        {r.entry
                                                                            .amount ??
                                                                            '-'}
                                                                    </td>
                                                                    <td>
                                                                        <button
                                                                            type="button"
                                                                            className="primary-button"
                                                                            disabled={
                                                                                busyPostEntryId !==
                                                                                null
                                                                            }
                                                                            onClick={() => {
                                                                                // Non-cancelling pair →
                                                                                // prompt for a fee
                                                                                // account first.
                                                                                const residual =
                                                                                    transferModalResidual(
                                                                                        r,
                                                                                    );
                                                                                if (
                                                                                    residual !==
                                                                                    null
                                                                                ) {
                                                                                    setTransferModalFeePrompt(
                                                                                        {
                                                                                            candidate:
                                                                                                r,
                                                                                            residual,
                                                                                        },
                                                                                    );
                                                                                    return;
                                                                                }
                                                                                void handleLinkTransferFromModal(
                                                                                    r,
                                                                                );
                                                                            }}
                                                                        >
                                                                            Link
                                                                        </button>
                                                                    </td>
                                                                </tr>
                                                            ),
                                                        )
                                                    )}
                                                </tbody>
                                            </table>
                                        </div>
                                    )}
                                </div>
                            </div>
                        )}
                        {splitModalEntryId !== null && (
                            <div
                                className="modal-overlay"
                                onClick={() => {
                                    setSplitModalEntryId(null);
                                }}
                            >
                                <div
                                    className="modal-dialog"
                                    onClick={(e) => {
                                        e.stopPropagation();
                                    }}
                                >
                                    <div className="modal-header">
                                        <h3>Split Transaction</h3>
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                setSplitModalEntryId(null);
                                            }}
                                        >
                                            Close
                                        </button>
                                    </div>
                                    <p className="status">
                                        Assign the full amount across
                                        counterpart accounts. Leave the last
                                        row&apos;s amount blank to let hledger
                                        infer the remainder.
                                    </p>
                                    <table className="ledger-table">
                                        <thead>
                                            <tr>
                                                <th>Account</th>
                                                <th>Amount</th>
                                                <th></th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {splitDraftRows.map((row, i) => (
                                                <tr key={i}>
                                                    <td>
                                                        <input
                                                            type="text"
                                                            placeholder="Expenses:Food"
                                                            value={row.account}
                                                            onChange={(e) => {
                                                                const v =
                                                                    e.target
                                                                        .value;
                                                                setSplitDraftRows(
                                                                    (cur) =>
                                                                        cur.map(
                                                                            (
                                                                                r,
                                                                                j,
                                                                            ) =>
                                                                                j ===
                                                                                i
                                                                                    ? {
                                                                                          ...r,
                                                                                          account:
                                                                                              v,
                                                                                      }
                                                                                    : r,
                                                                        ),
                                                                );
                                                            }}
                                                        />
                                                    </td>
                                                    <td>
                                                        <input
                                                            type="text"
                                                            placeholder={
                                                                i ===
                                                                splitDraftRows.length -
                                                                    1
                                                                    ? '(remainder)'
                                                                    : '0.00 USD'
                                                            }
                                                            value={row.amount}
                                                            onChange={(e) => {
                                                                const v =
                                                                    e.target
                                                                        .value;
                                                                setSplitDraftRows(
                                                                    (cur) =>
                                                                        cur.map(
                                                                            (
                                                                                r,
                                                                                j,
                                                                            ) =>
                                                                                j ===
                                                                                i
                                                                                    ? {
                                                                                          ...r,
                                                                                          amount: v,
                                                                                      }
                                                                                    : r,
                                                                        ),
                                                                );
                                                            }}
                                                        />
                                                    </td>
                                                    <td>
                                                        {splitDraftRows.length >
                                                            2 && (
                                                            <button
                                                                type="button"
                                                                className="ghost-button"
                                                                onClick={() => {
                                                                    setSplitDraftRows(
                                                                        (cur) =>
                                                                            cur.filter(
                                                                                (
                                                                                    _,
                                                                                    j,
                                                                                ) =>
                                                                                    j !==
                                                                                    i,
                                                                            ),
                                                                    );
                                                                }}
                                                            >
                                                                ×
                                                            </button>
                                                        )}
                                                    </td>
                                                </tr>
                                            ))}
                                        </tbody>
                                    </table>
                                    <div className="pipeline-row-actions">
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                setSplitDraftRows((cur) => [
                                                    ...cur,
                                                    {
                                                        account: '',
                                                        amount: '',
                                                    },
                                                ]);
                                            }}
                                        >
                                            + Add row
                                        </button>
                                        <button
                                            type="button"
                                            className="primary-button"
                                            onClick={() => {
                                                void handlePipelinePostSplit(
                                                    splitModalEntryId,
                                                    splitDraftRows,
                                                );
                                            }}
                                        >
                                            Post Split
                                        </button>
                                    </div>
                                </div>
                            </div>
                        )}
                    </div>
                )}
                {pipelineStatus === null ? null : (
                    <p className="status">{pipelineStatus}</p>
                )}
            </section>
            {(lightboxLoading ||
                lightboxSrc !== null ||
                lightboxError !== null) && (
                <div
                    className="modal-overlay"
                    onClick={closeLightbox}
                    role="dialog"
                    aria-modal="true"
                    aria-label={lightboxFilename ?? 'Attachment'}
                >
                    <div
                        className="modal-dialog attachment-lightbox"
                        onClick={(e) => {
                            e.stopPropagation();
                        }}
                    >
                        <div className="modal-header">
                            <h3>{lightboxFilename}</h3>
                            <button
                                type="button"
                                onClick={closeLightbox}
                                className="ghost-button"
                            >
                                Close
                            </button>
                        </div>
                        {lightboxLoading ? (
                            <p className="status">Loading…</p>
                        ) : lightboxError !== null ? (
                            <p className="status">{lightboxError}</p>
                        ) : lightboxSrc !== null ? (
                            <img
                                src={lightboxSrc}
                                alt={lightboxFilename ?? 'attachment'}
                                className="attachment-image"
                            />
                        ) : null}
                    </div>
                </div>
            )}
            {textViewContent !== null && (
                <div
                    className="modal-overlay"
                    onClick={() => {
                        setTextViewContent(null);
                    }}
                    role="dialog"
                    aria-modal="true"
                    aria-label={textViewFilename ?? 'Document'}
                >
                    <div
                        className="modal-dialog attachment-lightbox"
                        onClick={(e) => {
                            e.stopPropagation();
                        }}
                    >
                        <div className="modal-header">
                            <h3>{textViewFilename}</h3>
                            <button
                                type="button"
                                onClick={() => {
                                    setTextViewContent(null);
                                }}
                                className="ghost-button"
                            >
                                Close
                            </button>
                        </div>
                        <pre
                            style={{
                                overflow: 'auto',
                                maxHeight: 'calc(90vh - 4rem)',
                                margin: 0,
                                padding: '0.5rem',
                            }}
                        >
                            {textViewContent}
                        </pre>
                    </div>
                </div>
            )}
        </div>
    );
}

import { summarizeBulkRecategorize } from '../categorize-utils.ts';
import { Modal } from './Modal.tsx';

// Shared confirmation modal for recategorizing a selection of rows whose
// current accounts differ. Previously near-duplicated in TransactionsTable.tsx
// and TransactionsTab.tsx (they differed only in confirm side effects and
// ascii-vs-unicode glyphs). Caller-specific side effects stay in onConfirm.
export function BulkRecategorizeConfirmModal({
    newAccount,
    entries,
    onCancel,
    onConfirm,
}: {
    newAccount: string;
    entries: { oldAccount: string }[];
    onCancel: () => void;
    onConfirm: () => void;
}) {
    return (
        <Modal onClose={onCancel} ariaLabel="Confirm bulk recategorize">
            <div className="modal-header">
                <h3>Confirm bulk recategorize</h3>
                <button
                    type="button"
                    className="ghost-button"
                    onClick={onCancel}
                >
                    Close
                </button>
            </div>
            <p>
                The selected rows have different current accounts. All will be
                changed to <strong>{newAccount}</strong>:
            </p>
            <ul>
                {summarizeBulkRecategorize(entries).map(
                    ({ oldAccount, count }) => (
                        <li key={oldAccount}>
                            {count} × {oldAccount} → {newAccount}
                        </li>
                    ),
                )}
            </ul>
            <div
                style={{
                    display: 'flex',
                    gap: '0.5rem',
                    justifyContent: 'flex-end',
                }}
            >
                <button
                    type="button"
                    className="ghost-button"
                    onClick={onCancel}
                >
                    Cancel
                </button>
                <button
                    type="button"
                    className="ghost-button"
                    onClick={onConfirm}
                >
                    Confirm
                </button>
            </div>
        </Modal>
    );
}

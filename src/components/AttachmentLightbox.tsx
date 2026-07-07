// Shared image-attachment lightbox modal, previously duplicated verbatim in
// TransactionsTable.tsx and PipelineTab.tsx. Its state/fetch logic lives in
// useAttachmentLightbox.ts.
import { Modal } from './Modal.tsx';

export function AttachmentLightbox({
    filename,
    src,
    loading,
    error,
    onClose,
}: {
    filename: string | null;
    src: string | null;
    loading: boolean;
    error: string | null;
    onClose: () => void;
}) {
    if (!(loading || src !== null || error !== null)) return null;
    return (
        <Modal
            onClose={onClose}
            ariaLabel={filename ?? 'Attachment'}
            dialogClassName="modal-dialog attachment-lightbox"
        >
            <div className="modal-header">
                <h3>{filename}</h3>
                <button
                    type="button"
                    onClick={onClose}
                    className="ghost-button"
                >
                    Close
                </button>
            </div>
            {loading ? (
                <p className="status">Loading…</p>
            ) : error !== null ? (
                <p className="status">{error}</p>
            ) : src !== null ? (
                <img
                    src={src}
                    alt={filename ?? 'attachment'}
                    className="attachment-image"
                />
            ) : null}
        </Modal>
    );
}

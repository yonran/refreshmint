// Shared image-attachment lightbox modal, previously duplicated verbatim in
// TransactionsTable.tsx and PipelineTab.tsx. Its state/fetch logic lives in
// useAttachmentLightbox.ts.

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
        <div
            className="modal-overlay"
            onClick={onClose}
            role="dialog"
            aria-modal="true"
            aria-label={filename ?? 'Attachment'}
        >
            <div
                className="modal-dialog attachment-lightbox"
                onClick={(e) => {
                    e.stopPropagation();
                }}
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
            </div>
        </div>
    );
}

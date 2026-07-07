import { useEffect, type ReactNode } from 'react';
import { shouldCloseOnKey } from '../status-utils.ts';

// Shared modal shell: a full-screen overlay plus a centered dialog, closable by
// Escape and/or an overlay click. Previously each modal hand-rolled the
// overlay/dialog/stopPropagation markup and none closed on Escape. Adopters
// pass only their inner content.
//
// Escape handling listens on window in the bubble phase and ignores events
// whose default was already prevented (see shouldCloseOnKey), so an inner
// control that consumes the first Escape — e.g. AccountInput dismissing its
// suggestions — does not also close the modal on the same keystroke. No focus
// trap yet (deferred).
export function Modal({
    onClose,
    closeOnEscape = true,
    closeOnOverlayClick = true,
    ariaLabel,
    dialogClassName = 'modal-dialog',
    children,
}: {
    onClose: () => void;
    closeOnEscape?: boolean;
    closeOnOverlayClick?: boolean;
    ariaLabel?: string | undefined;
    dialogClassName?: string;
    children: ReactNode;
}) {
    useEffect(() => {
        if (!closeOnEscape) return;
        function onKeyDown(e: KeyboardEvent) {
            if (shouldCloseOnKey(e.key, e.defaultPrevented, closeOnEscape)) {
                onClose();
            }
        }
        window.addEventListener('keydown', onKeyDown);
        return () => {
            window.removeEventListener('keydown', onKeyDown);
        };
    }, [closeOnEscape, onClose]);

    return (
        <div
            className="modal-overlay"
            onClick={
                closeOnOverlayClick
                    ? () => {
                          onClose();
                      }
                    : undefined
            }
        >
            <div
                className={dialogClassName}
                role="dialog"
                aria-modal="true"
                aria-label={ariaLabel}
                onClick={(e) => {
                    e.stopPropagation();
                }}
            >
                {children}
            </div>
        </div>
    );
}

import { useEffect } from 'react';

export type StatusLevel = 'error' | 'info' | 'busy';

// Shared status banner used across tabs to surface transient action results
// (errors, progress, confirmations) in a single consistent place. Error
// styling reuses `.query-error`; info/busy reuse `.status`; `.status-banner`
// adds the dismiss/action layout. See src/status-utils.ts for the
// message-classification helper that picks the level for free-form messages.
export function StatusBanner({
    level,
    message,
    onDismiss,
    action,
    autoDismissMs,
}: {
    level: StatusLevel;
    message: string;
    onDismiss?: () => void;
    action?: { label: string; onClick: () => void };
    autoDismissMs?: number;
}) {
    useEffect(() => {
        if (autoDismissMs === undefined || onDismiss === undefined) return;
        const timer = setTimeout(onDismiss, autoDismissMs);
        return () => {
            clearTimeout(timer);
        };
        // Re-arm when the message changes so a new banner gets its own timer.
    }, [autoDismissMs, onDismiss, message]);

    const className =
        level === 'error'
            ? 'query-error status-banner'
            : 'status status-banner';

    return (
        <div
            className={className}
            role={level === 'error' ? 'alert' : 'status'}
        >
            <span className="status-banner-message">{message}</span>
            {action !== undefined && (
                <button
                    type="button"
                    className="ghost-button status-banner-action"
                    onClick={action.onClick}
                >
                    {action.label}
                </button>
            )}
            {onDismiss !== undefined && (
                <button
                    type="button"
                    className="ghost-button status-banner-dismiss"
                    aria-label="Dismiss"
                    onClick={onDismiss}
                >
                    ✕
                </button>
            )}
        </div>
    );
}

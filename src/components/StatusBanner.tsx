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
    // `| undefined` so callers can forward optional fields from a nullable
    // status object under exactOptionalPropertyTypes. `disabled` lets a caller
    // gate the action while its handler is already in flight (e.g. an Undo
    // running against the GL).
    action?:
        | { label: string; onClick: () => void; disabled?: boolean }
        | undefined;
    autoDismissMs?: number | undefined;
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
                    disabled={action.disabled ?? false}
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

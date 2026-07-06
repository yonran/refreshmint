import { useState } from 'react';
import {
    checkAccountTypeChange,
    getAccountSuggestions,
} from '../search-utils.ts';

export function AccountInput({
    value,
    onChange,
    onKeyDown,
    accounts,
    oldAccount,
    autoFocus,
    placeholder = 'Account name…',
    disabled,
}: {
    value: string;
    onChange: (value: string) => void;
    onKeyDown?: (e: React.KeyboardEvent<HTMLInputElement>) => void;
    accounts: string[];
    /** Old account(s) being replaced — used to filter suggestions and show type-change warning. */
    oldAccount?: string | string[];
    autoFocus?: boolean;
    placeholder?: string;
    disabled?: boolean;
}) {
    const [suggestions, setSuggestions] = useState<string[]>([]);
    const [activeIndex, setActiveIndex] = useState(-1);

    const warning =
        value.trim() !== ''
            ? checkAccountTypeChange(oldAccount ?? [], value.trim())
            : null;

    function computeSuggestions(draft: string) {
        setSuggestions(getAccountSuggestions(draft, accounts, oldAccount));
        setActiveIndex(-1);
    }

    function applyCompletion(sug: string) {
        onChange(sug);
        // Re-compute for the chosen value (e.g. "Expenses:" → show sub-accounts)
        const next = getAccountSuggestions(sug, accounts, oldAccount);
        setSuggestions(next);
        setActiveIndex(-1);
    }

    // ArrowDown/Up navigate, Enter/Tab (with active item) select, Escape dismisses.
    // Unhandled keys pass through to onKeyDown so the parent's Enter (commit)
    // and Escape (cancel editing) still fire when no suggestion is active.
    function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
        if (suggestions.length === 0) {
            onKeyDown?.(e);
            return;
        }
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            setActiveIndex((i) => Math.min(i + 1, suggestions.length - 1));
        } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            setActiveIndex((i) => Math.max(i - 1, 0));
        } else if ((e.key === 'Enter' || e.key === 'Tab') && activeIndex >= 0) {
            e.preventDefault();
            const sug = suggestions[activeIndex];
            if (sug !== undefined) applyCompletion(sug);
        } else if (e.key === 'Escape') {
            // First Escape dismisses suggestions; second Escape reaches parent.
            setSuggestions([]);
            setActiveIndex(-1);
        } else {
            onKeyDown?.(e);
        }
    }

    return (
        <div className="account-input-wrap">
            <input
                type="text"
                value={value}
                placeholder={placeholder}
                autoFocus={autoFocus}
                disabled={disabled}
                onFocus={(e) => {
                    e.target.select();
                    computeSuggestions(e.target.value);
                }}
                onChange={(e) => {
                    onChange(e.target.value);
                    computeSuggestions(e.target.value);
                }}
                onKeyDown={handleKeyDown}
                onBlur={() => {
                    // Allow mousedown on a suggestion item to fire before blur
                    // closes the list.
                    setTimeout(() => {
                        setSuggestions([]);
                        setActiveIndex(-1);
                    }, 150);
                }}
            />
            {suggestions.length > 0 && (
                <div className="account-suggestions" role="listbox">
                    {suggestions.map((sug, i) => (
                        <div
                            key={sug}
                            className={`ac-item${i === activeIndex ? ' active' : ''}`}
                            role="option"
                            aria-selected={i === activeIndex}
                            onMouseDown={(e) => {
                                e.preventDefault(); // keep input focus
                                applyCompletion(sug);
                            }}
                        >
                            {sug}
                        </div>
                    ))}
                </div>
            )}
            {warning != null && warning !== '' && (
                <div className="account-warning">{warning}</div>
            )}
        </div>
    );
}

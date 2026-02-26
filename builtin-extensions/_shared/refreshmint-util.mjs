function inspect(value, options = undefined) {
    const maxDepth =
        typeof options === 'object' &&
        options !== null &&
        Number.isFinite(options.depth)
            ? Math.max(0, Math.floor(options.depth))
            : 6;
    const seen = new WeakSet();

    const quoteString = (text) => JSON.stringify(String(text));

    function formatPrimitive(v) {
        if (v === null) return 'null';
        const t = typeof v;
        if (t === 'undefined') return 'undefined';
        if (t === 'string') return quoteString(v);
        if (t === 'number' || t === 'boolean' || t === 'bigint')
            return String(v);
        if (t === 'symbol') return String(v);
        if (t === 'function') return `[Function${v.name ? `: ${v.name}` : ''}]`;
        return null;
    }

    function formatAny(v, depth) {
        const primitive = formatPrimitive(v);
        if (primitive != null) return primitive;

        if (v instanceof Error) {
            return formatError(v, depth);
        }

        if (seen.has(v)) return '[Circular]';
        if (depth >= maxDepth) {
            if (Array.isArray(v)) return '[Array]';
            const ctorName =
                v != null &&
                v.constructor != null &&
                typeof v.constructor.name === 'string'
                    ? v.constructor.name
                    : 'Object';
            return `[${ctorName}]`;
        }

        seen.add(v);

        if (Array.isArray(v)) {
            const parts = v.map((item) => formatAny(item, depth + 1));
            return `[ ${parts.join(', ')} ]`;
        }

        const ctorName =
            v != null &&
            v.constructor != null &&
            typeof v.constructor.name === 'string'
                ? v.constructor.name
                : 'Object';
        const keys = Reflect.ownKeys(v);
        const entries = [];
        for (const key of keys) {
            const keyText = typeof key === 'string' ? key : `[${String(key)}]`;
            let valueText;
            try {
                valueText = formatAny(v[key], depth + 1);
            } catch (err) {
                valueText = `[Thrown: ${String(err)}]`;
            }
            entries.push(`${keyText}: ${valueText}`);
        }
        if (entries.length === 0) {
            return ctorName === 'Object' ? '{}' : `${ctorName} {}`;
        }
        const body = `{ ${entries.join(', ')} }`;
        return ctorName === 'Object' ? body : `${ctorName} ${body}`;
    }

    function formatError(err, depth) {
        const name = err.name || 'Error';
        const message = err.message || '';
        const summary = message ? `${name}: ${message}` : name;
        let header;
        if (typeof err.stack === 'string' && err.stack.trim() !== '') {
            const stack = err.stack;
            header = stack.startsWith(summary) ? stack : `${summary}\n${stack}`;
        } else {
            header = summary;
        }

        if (depth >= maxDepth) return header;

        const details = [];
        if ('cause' in err) {
            details.push(`cause: ${formatAny(err.cause, depth + 1)}`);
        }
        for (const key of Reflect.ownKeys(err)) {
            if (key === 'cause') continue;
            const keyText = typeof key === 'string' ? key : `[${String(key)}]`;
            let valueText;
            try {
                valueText = formatAny(err[key], depth + 1);
            } catch (innerErr) {
                valueText = `[Thrown: ${String(innerErr)}]`;
            }
            details.push(`${keyText}: ${valueText}`);
        }

        if (details.length === 0) return header;
        return `${header}\n{ ${details.join(', ')} }`;
    }

    return formatAny(value, 0);
}

export { inspect };
export default { inspect };

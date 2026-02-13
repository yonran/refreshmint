// Workaround for Plugin type incompatibility with exactOptionalPropertyTypes
// See: https://github.com/eslint/eslint/issues/20286
declare module 'eslint-plugin-react-hooks' {
    import type { ESLint, Linter } from 'eslint';

    const plugin: ESLint.Plugin & {
        configs: {
            recommended: {
                plugins: string[];
                rules: Linter.RulesRecord;
            };
            'recommended-latest': {
                plugins: string[];
                rules: Linter.RulesRecord;
            };
        };
    };

    export default plugin;
}

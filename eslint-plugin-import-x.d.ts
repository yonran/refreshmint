// Workaround for flatConfigs type incompatibility with exactOptionalPropertyTypes
// See: https://github.com/import-js/eslint-plugin-import/issues/3169
// See: https://github.com/eslint/eslint/issues/20286
declare module 'eslint-plugin-import-x' {
    import type { ESLint, Linter } from 'eslint';

    export const flatConfigs: {
        recommended: Linter.Config;
        errors: Linter.Config;
        warnings: Linter.Config;
        'stage-0': Linter.Config;
        react: Linter.Config;
        'react-native': Linter.Config;
        electron: Linter.Config;
        typescript: Linter.Config;
        [key: string]: Linter.Config | undefined;
    };

    const plugin: ESLint.Plugin & {
        flatConfigs: typeof flatConfigs;
    };

    export default plugin;
}

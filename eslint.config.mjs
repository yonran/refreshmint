import fs from 'node:fs'
import path from 'node:path'
import js from '@eslint/js'
import globals from 'globals'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import tseslint from 'typescript-eslint'
import { defineConfig, globalIgnores } from 'eslint/config'
import importPlugin from 'eslint-plugin-import-x'

const vitePublicResolver = {
  name: 'vite-public',
  resolver: {
    resolveImport(modulePath) {
      if (!modulePath.startsWith('/')) {
        return undefined
      }

      const resolvedPath = path.resolve(import.meta.dirname, 'public', modulePath.slice(1))

      return fs.existsSync(resolvedPath) ? resolvedPath : undefined
    },
  },
}

export default defineConfig(
  globalIgnores(['dist', 'src-tauri/target']),
  js.configs.recommended,

  // eslint-disable-next-line import-x/no-named-as-default-member
  importPlugin.flatConfigs.recommended,
  // eslint-disable-next-line import-x/no-named-as-default-member
  importPlugin.flatConfigs.typescript,
  reactHooks.configs.flat.recommended,
  reactRefresh.configs.vite,
  // eslint-disable-next-line import-x/no-named-as-default-member
  ...tseslint.configs.strictTypeChecked,
  {
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
      parserOptions: {
        projectService: true,
        tsconfigDirName: import.meta.dirname,
      },
    },
    settings: {
      'import-x/resolver': [
        vitePublicResolver,
        { typescript: true },
        { node: true },
      ],
    },
    rules: {
      // disallow unused variables, except when they start with _
      // see example config in
      // https://typescript-eslint.io/rules/no-unused-vars/#benefits-over-typescript
      '@typescript-eslint/no-unused-vars': [
        'error',
        {
          args: 'all',
          argsIgnorePattern: '^_',
          caughtErrors: 'all',
          caughtErrorsIgnorePattern: '^_',
          destructuredArrayIgnorePattern: '^_',
          varsIgnorePattern: '^_',
          ignoreRestSiblings: true,
        },
      ],
      // modify the typescript-eslint recommended config:
      // allow literals without casting to string e.g. `${true} ${1}` etc.
      // since it is a bad habit to wrap things in String(); you might wrap an object accidentally
      '@typescript-eslint/restrict-template-expressions': ['error', {
        allowBoolean: true,
        allowNullish: true,
        allowNumber: true,
      }],
      // claude often does if (someStr) or if (someNum) when it should do if (someStr != null)
      '@typescript-eslint/strict-boolean-expressions': [
        'error',
        {
          allowNullableObject: true,
        }
      ],
      '@typescript-eslint/consistent-type-imports': 'error',
      '@typescript-eslint/no-unsafe-type-assertion': 'error',
      'import-x/no-cycle': 'error',
    },
  },
  {
    files: ['**/*.mjs'],
    // eslint-disable-next-line import-x/no-named-as-default-member
    ...tseslint.configs.disableTypeChecked,
  },
  {
    ignores: ['dist/**'],
  },
);

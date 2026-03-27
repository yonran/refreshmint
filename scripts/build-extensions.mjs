import fs from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

import { nodeResolve } from '@rollup/plugin-node-resolve';
import { rollup } from 'rollup';
import {
    DiagnosticCategory,
    ModuleKind,
    ScriptTarget,
    formatDiagnosticsWithColorAndContext,
    transpileModule,
} from 'typescript';

const SCRIPT_PATH = fileURLToPath(import.meta.url);
const REPO_ROOT = path.resolve(path.dirname(SCRIPT_PATH), '..');
const BUILTIN_EXTENSIONS_ROOT = path.join(REPO_ROOT, 'builtin-extensions');
const SCRIPT_EXTENSIONS = new Set(['.mjs', '.js', '.mts', '.ts']);

async function main() {
    const args = parseArgs(process.argv.slice(2));

    if (args.builtinOutDir != null) {
        await buildBuiltinExtensions(args.builtinOutDir);
        return;
    }

    if (args.extensionDir == null || args.outDir == null) {
        throw new Error(
            'usage: node scripts/build-extensions.mjs --extension-dir <dir> --out-dir <dir>\n' +
                '   or: node scripts/build-extensions.mjs --builtin-out-dir <dir>',
        );
    }

    await buildExtension(args.extensionDir, args.outDir);
}

function parseArgs(argv) {
    const parsed = {
        builtinOutDir: null,
        extensionDir: null,
        outDir: null,
    };

    for (let index = 0; index < argv.length; index += 1) {
        const arg = argv[index];
        const value = argv[index + 1];
        switch (arg) {
            case '--builtin-out-dir':
                parsed.builtinOutDir = requireValue(arg, value);
                index += 1;
                break;
            case '--extension-dir':
                parsed.extensionDir = requireValue(arg, value);
                index += 1;
                break;
            case '--out-dir':
                parsed.outDir = requireValue(arg, value);
                index += 1;
                break;
            default:
                throw new Error(`unknown argument: ${arg}`);
        }
    }

    return parsed;
}

function requireValue(flag, value) {
    if (value == null) {
        throw new Error(`missing value for ${flag}`);
    }
    return value;
}

async function buildBuiltinExtensions(builtinOutDir) {
    const entries = await fs.readdir(BUILTIN_EXTENSIONS_ROOT, {
        withFileTypes: true,
    });
    await fs.rm(builtinOutDir, { recursive: true, force: true });
    await fs.mkdir(builtinOutDir, { recursive: true });

    for (const entry of entries) {
        if (!entry.isDirectory() || entry.name.startsWith('_')) {
            continue;
        }
        const extensionDir = path.join(BUILTIN_EXTENSIONS_ROOT, entry.name);
        const manifestPath = path.join(extensionDir, 'manifest.json');
        try {
            await fs.access(manifestPath);
        } catch {
            continue;
        }
        await buildExtension(
            extensionDir,
            path.join(builtinOutDir, entry.name),
        );
    }
}

async function buildExtension(extensionDir, outDir) {
    const sourceDir = path.resolve(extensionDir);
    const outputDir = path.resolve(outDir);
    const manifestPath = path.join(sourceDir, 'manifest.json');
    const manifest = JSON.parse(await fs.readFile(manifestPath, 'utf8'));
    const sourceDriver = manifest.driver ?? 'driver.mjs';
    const sourceExtract = manifest.extract ?? null;
    const entries = [];

    if (await pathExists(path.join(sourceDir, sourceDriver))) {
        entries.push({
            alias: 'driver',
            source: sourceDriver,
            manifestKey: 'driver',
        });
    }
    if (
        typeof sourceExtract === 'string' &&
        SCRIPT_EXTENSIONS.has(path.extname(sourceExtract)) &&
        (await pathExists(path.join(sourceDir, sourceExtract)))
    ) {
        entries.push({
            alias: 'extract',
            source: sourceExtract,
            manifestKey: 'extract',
        });
    }

    await fs.rm(outputDir, { recursive: true, force: true });
    await fs.mkdir(outputDir, { recursive: true });

    if (entries.length > 0) {
        await buildScriptEntries(
            sourceDir,
            path.join(outputDir, 'dist'),
            entries,
        );
    }

    const outputManifest = { ...manifest };
    if (entries.some((entry) => entry.manifestKey === 'driver')) {
        outputManifest.driver = 'dist/driver.mjs';
    }
    if (entries.some((entry) => entry.manifestKey === 'extract')) {
        outputManifest.extract = 'dist/extract.mjs';
    }
    await fs.writeFile(
        path.join(outputDir, 'manifest.json'),
        `${JSON.stringify(outputManifest, null, 4)}\n`,
    );

    if (typeof manifest.rules === 'string') {
        await copyRelativeFile(sourceDir, outputDir, manifest.rules);
    }
}

async function buildScriptEntries(sourceDir, distDir, entries) {
    const input = Object.fromEntries(
        entries.map((entry) => [
            entry.alias,
            path.join(sourceDir, entry.source),
        ]),
    );
    const bundle = await rollup({
        input,
        plugins: [
            nodeResolve({
                extensions: ['.mjs', '.js', '.mts', '.ts'],
                exportConditions: ['browser', 'import', 'default'],
                mainFields: ['module', 'main'],
                preferBuiltins: false,
            }),
            refreshmintTypeScriptPlugin(),
        ],
        treeshake: false,
    });

    try {
        await bundle.write({
            dir: distDir,
            format: 'esm',
            sourcemap: false,
            preserveModules: true,
            preserveModulesRoot: sourceDir,
            entryFileNames: '[name].mjs',
            chunkFileNames: '[name]-[hash].mjs',
        });
    } finally {
        await bundle.close();
    }
}

function refreshmintTypeScriptPlugin() {
    return {
        name: 'refreshmint-typescript',
        transform(code, id) {
            if (!id.endsWith('.ts') && !id.endsWith('.mts')) {
                return null;
            }
            const result = transpileModule(code, {
                compilerOptions: {
                    allowImportingTsExtensions: true,
                    module: ModuleKind.ESNext,
                    rewriteRelativeImportExtensions: true,
                    target: ScriptTarget.ESNext,
                    verbatimModuleSyntax: true,
                },
                fileName: id,
                reportDiagnostics: true,
            });
            const diagnostics = result.diagnostics ?? [];
            const nonSuggestion = diagnostics.filter(
                (diagnostic) =>
                    diagnostic.category !== DiagnosticCategory.Suggestion &&
                    diagnostic.category !== DiagnosticCategory.Message,
            );
            if (nonSuggestion.length > 0) {
                const rendered = formatDiagnosticsWithColorAndContext(
                    nonSuggestion,
                    {
                        getCanonicalFileName: (name) => name,
                        getCurrentDirectory: () => process.cwd(),
                        getNewLine: () => '\n',
                    },
                );
                throw new Error(`failed to transpile ${id}:\n${rendered}`);
            }
            return {
                code: result.outputText,
                map: null,
            };
        },
    };
}

async function copyRelativeFile(sourceDir, outputDir, relativePath) {
    const sourcePath = path.join(sourceDir, relativePath);
    const outputPath = path.join(outputDir, relativePath);
    await fs.mkdir(path.dirname(outputPath), { recursive: true });
    await fs.copyFile(sourcePath, outputPath);
}

async function pathExists(targetPath) {
    try {
        await fs.access(targetPath);
        return true;
    } catch {
        return false;
    }
}

main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
});

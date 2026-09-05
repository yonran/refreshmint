import { copyFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import process from 'node:process';

const debug = process.argv.includes('--debug');
/**
 * @param {string | undefined} value
 * @returns {string | undefined}
 */
const nonEmpty = (value) =>
    value !== undefined && value.length > 0 ? value : undefined;
const explicitTarget =
    nonEmpty(process.env.TAURI_ENV_TARGET_TRIPLE) ??
    nonEmpty(process.env.CARGO_BUILD_TARGET) ??
    nonEmpty(process.env.npm_config_target);

const rustc = spawnSync('rustc', ['-vV'], { encoding: 'utf8' });
if (rustc.status !== 0) {
    throw new Error(
        rustc.stderr.length > 0 ? rustc.stderr : 'failed to run rustc -vV',
    );
}
const host = rustc.stdout.match(/^host: (.+)$/m)?.[1];
const target = explicitTarget ?? host;
if (target === undefined || target.length === 0) {
    throw new Error('could not determine Rust target triple');
}

const cargoArgs = [
    'build',
    '--package',
    'refreshmint-scraper-runtime',
    '--bin',
    'scraper-worker',
    '--bin',
    'refreshmint-mcp',
];
if (!debug) cargoArgs.push('--release');
if (explicitTarget !== undefined) cargoArgs.push('--target', explicitTarget);

const cargo = spawnSync('cargo', cargoArgs, {
    cwd: resolve('src-tauri'),
    stdio: 'inherit',
});
if (cargo.status !== 0) {
    process.exit(cargo.status ?? 1);
}

const executableSuffix = target.includes('windows') ? '.exe' : '';
const profile = debug ? 'debug' : 'release';
const cargoTargetDir = nonEmpty(process.env.CARGO_TARGET_DIR);
const targetRoot =
    cargoTargetDir !== undefined
        ? resolve(cargoTargetDir)
        : resolve('src-tauri/target');
const builtRoot =
    explicitTarget !== undefined
        ? resolve(targetRoot, target, profile)
        : resolve(targetRoot, profile);
const destinationRoot = resolve('src-tauri/binaries');
mkdirSync(destinationRoot, { recursive: true });

for (const [builtName, bundledName] of [
    ['scraper-worker', 'refreshmint-scraper-worker'],
    ['refreshmint-mcp', 'refreshmint-mcp-server'],
]) {
    copyFileSync(
        resolve(builtRoot, `${builtName}${executableSuffix}`),
        resolve(destinationRoot, `${bundledName}-${target}${executableSuffix}`),
    );
}

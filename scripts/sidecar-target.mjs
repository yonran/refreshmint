/**
 * Decide how `prepare-rust-sidecars.mjs` invokes cargo for a requested target.
 *
 * Passing `--target <host triple>` makes cargo build into
 * `target/<triple>/<profile>/` instead of `target/<profile>/`, a separate
 * artifact tree from every plain `cargo build`/`cargo test`/`app debug start`
 * on the machine. `tauri dev` always sets `TAURI_ENV_TARGET_TRIPLE`, so the
 * GUI's scraper-worker and the CLI's `target/debug/scraper-worker` used to be
 * two distinct compiles with two distinct code hashes, and macOS Keychain
 * asked "Always Allow" once for each and again after every rebuild. Only pass
 * `--target` for a genuine cross-compile so the host build shares one tree.
 *
 * @param {{ explicitTarget: string | undefined, host: string }} args
 * @returns {{ target: string, cargoTargetArg: string | undefined, builtSubdir: string[] }}
 *   `target` is the triple used for the bundled `<name>-<triple>` filename,
 *   `cargoTargetArg` is what to pass after `--target` (undefined for none),
 *   and `builtSubdir` is the path cargo puts artifacts under, relative to the
 *   target root and before the profile directory (`[]` for host builds,
 *   `[triple]` for cross builds).
 */
export function resolveSidecarTarget({ explicitTarget, host }) {
    const target = explicitTarget ?? host;
    const isCross = explicitTarget !== undefined && explicitTarget !== host;
    return {
        target,
        cargoTargetArg: isCross ? explicitTarget : undefined,
        builtSubdir: isCross ? [target] : [],
    };
}

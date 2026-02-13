#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';

/**
 * Gets the path to git's user excludes file.
 * Tries `git config core.excludesFile` first, falls back to default location.
 * @returns {string} Path to the user-level git excludes file
 */
export function getGitUserIgnoreFile(): string {
    try {
        const result = execFileSync(
            'git',
            ['config', '--get', 'core.excludesFile'],
            {
                encoding: 'utf8',
            },
        ).trim();
        if (result) {
            return result;
        }
    } catch (_e) {
        // Config not set, use default
    }

    // Default per https://git-scm.com/docs/gitignore
    // "If $XDG_CONFIG_HOME is either not set or empty, $HOME/.config/git/ignore is used instead."
    const xdgConfigHome = process.env['XDG_CONFIG_HOME'];
    const xdgConfig =
        xdgConfigHome != null && xdgConfigHome !== ''
            ? xdgConfigHome
            : path.join(os.homedir(), '.config');
    return path.join(xdgConfig, 'git', 'ignore');
}

// When run as a script, output the path
if (import.meta.url === `file://${process.argv[1]}`) {
    console.log(getGitUserIgnoreFile());
}

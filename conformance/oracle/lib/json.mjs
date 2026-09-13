import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

/** Canonical JSON: recursively sorted keys, two-space indent, trailing newline. */
export function stringify(value) {
    return `${JSON.stringify(sortKeys(value), null, 2)}\n`;
}

export function sortKeys(value) {
    if (Array.isArray(value)) return value.map(sortKeys);
    if (value && typeof value === 'object') {
        return Object.fromEntries(Object.keys(value).sort().map(key => [key, sortKeys(value[key])]));
    }
    return value;
}

export function sha256(value) {
    return createHash('sha256').update(value).digest('hex');
}

export function readJson(path) {
    return JSON.parse(readFileSync(path, 'utf8'));
}

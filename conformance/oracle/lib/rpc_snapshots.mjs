// Shared helpers for the frozen transaction captures under ios-demo/Resources:
// JSON-RPC plumbing, base58 decoding, and the account-key/role reconstruction
// that turns a `getTransaction` result into ordered instruction metas.

import assert from 'node:assert/strict';

export { readJson, sha256, sortKeys, stringify } from './json.mjs';

export async function rpc(endpoint, method, params) {
    const payload = await post(endpoint, method, params);
    if (payload.error) throw new Error(`${method}: ${JSON.stringify(payload.error)}`);
    return payload.result;
}

export async function rpcWithContext(endpoint, method, params) {
    const result = await rpc(endpoint, method, params);
    assert.ok(result?.context, `${method} response has no context`);
    return result;
}

/** Public endpoints rate-limit aggressively; 429s are retried with backoff. */
async function post(endpoint, method, params) {
    let delay = 1000;
    for (let attempt = 0; ; attempt += 1) {
        const response = await fetch(endpoint, {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
        });
        if (response.status === 429 && attempt < 6) {
            const retryAfter = Number(response.headers.get('retry-after'));
            await new Promise(resolve => setTimeout(resolve, retryAfter > 0 ? retryAfter * 1000 : delay));
            delay = Math.min(delay * 2, 16_000);
            continue;
        }
        if (!response.ok) throw new Error(`${method}: HTTP ${response.status}`);
        return response.json();
    }
}

const BASE58_ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';

export function decodeBase58(value) {
    let bytes = [0];
    for (const character of value) {
        const digit = BASE58_ALPHABET.indexOf(character);
        if (digit < 0) throw new Error(`Invalid base58 character: ${character}`);
        let carry = digit;
        for (let index = 0; index < bytes.length; index += 1) {
            carry += bytes[index] * 58;
            bytes[index] = carry & 0xff;
            carry >>= 8;
        }
        while (carry > 0) {
            bytes.push(carry & 0xff);
            carry >>= 8;
        }
    }
    for (let index = 0; index < value.length - 1 && value[index] === '1'; index += 1) {
        bytes.push(0);
    }
    return Uint8Array.from(bytes.reverse());
}

export function encodeBase58(bytes) {
    let digits = [0];
    for (const byte of bytes) {
        let carry = byte;
        for (let index = 0; index < digits.length; index += 1) {
            carry += digits[index] << 8;
            digits[index] = carry % 58;
            carry = (carry / 58) | 0;
        }
        while (carry > 0) {
            digits.push(carry % 58);
            carry = (carry / 58) | 0;
        }
    }
    let output = '';
    for (const byte of bytes) {
        if (byte !== 0) break;
        output += '1';
    }
    for (let index = digits.length - 1; index >= 0; index -= 1) {
        output += BASE58_ALPHABET[digits[index]];
    }
    return output;
}

/** Static keys followed by the writable then readonly loaded addresses. */
export function completeAccountKeys(result) {
    const staticKeys = result.transaction.message.accountKeys.map(key =>
        typeof key === 'string' ? key : key.pubkey,
    );
    const loaded = result.meta?.loadedAddresses ?? { writable: [], readonly: [] };
    return [...staticKeys, ...loaded.writable, ...loaded.readonly];
}

export function instructionAccounts(result, instructionIndex) {
    const keys = completeAccountKeys(result);
    return result.transaction.message.instructions[instructionIndex].accounts.map(index => keys[index]);
}

/** Ordered metas of one top-level instruction with the fixture role vocabulary. */
export function instructionAccountsWithRoles(result, instructionIndex) {
    const message = result.transaction.message;
    const staticCount = message.accountKeys.length;
    const header = message.header;
    const target = message.instructions[instructionIndex];
    const keys = completeAccountKeys(result);
    const loadedWritableCount = result.meta?.loadedAddresses?.writable?.length ?? 0;
    return target.accounts.map(index => {
        const isSigner = index < header.numRequiredSignatures;
        let isWritable;
        if (index < staticCount) {
            if (isSigner) {
                isWritable = index < header.numRequiredSignatures - header.numReadonlySignedAccounts;
            } else {
                isWritable = index < staticCount - header.numReadonlyUnsignedAccounts;
            }
        } else {
            isWritable = index < staticCount + loadedWritableCount;
        }
        return {
            address: keys[index],
            role: isWritable
                ? (isSigner ? 'writableSigner' : 'writable')
                : (isSigner ? 'readonlySigner' : 'readonly'),
        };
    });
}

export function transactionVersion(result) {
    return result.version === 0 ? 'v0' : result.version;
}

// Frozen devnet captures for the Subscriptions & Allowances program.
//
//   node devnet_examples.mjs               verify the committed captures offline (CI)
//   node devnet_examples.mjs --discover    scan devnet for one transaction per wanted opcode (read-only, prints catalog entries)
//   node devnet_examples.mjs --update      refetch every catalog example and its account snapshots (network)
//
// The public devnet endpoint needs no key. Transfers read the frozen Mint
// through the IDL's account link; the other account snapshots document the
// captures without supplying scales to delegation approvals.

import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
    decodeBase58,
    encodeBase58,
    completeAccountKeys,
    instructionAccounts,
    instructionAccountsWithRoles,
    readJson,
    rpc,
    rpcWithContext,
    sha256,
    stringify,
    transactionVersion,
} from './lib/rpc_snapshots.mjs';

const FLAGS = { discover: '--discover', update: '--update' };
const args = process.argv.slice(2).filter(argument => argument !== '--');
if (args.some(argument => !Object.values(FLAGS).includes(argument) && !argument.startsWith('--max-transactions='))) {
    throw new Error(`Usage: node devnet_examples.mjs [${FLAGS.discover}|${FLAGS.update}] [--max-transactions=N]`);
}
const discover = args.includes(FLAGS.discover);
const update = args.includes(FLAGS.update);
const maxTransactions = Number(args.find(argument => argument.startsWith('--max-transactions='))?.split('=')[1] ?? 3000);

const oracleDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(oracleDir, '..', '..');
const resourcesDir = join(repoRoot, 'ios-demo', 'Resources', 'DevnetExamples');
const transactionsDir = join(resourcesDir, 'Transactions');
const catalogPath = join(resourcesDir, 'catalog.json');
const accountsPath = join(resourcesDir, 'accounts.json');
const provenancePath = join(resourcesDir, 'provenance.json');
const conformanceDir = join(oracleDir, '..', 'fixtures', 'subscriptions');
const casesPath = join(conformanceDir, 'cases.json');
const idlProvenancePath = join(conformanceDir, 'provenance.json');
const idlPath = join(conformanceDir, 'root.json');
const bundledIdlPath = join(repoRoot, 'ios-demo', 'Resources', 'Srf39', 'subscriptions-root.json');
const manifestPath = join(repoRoot, 'ios-demo', 'Resources', 'Srf39', 'srf39-manifest.json');
const registryPath = join(repoRoot, 'ios-demo', 'Resources', 'Srf39', 'token-registry-devnet.json');

const PROGRAM_ID = 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44';
const ENDPOINT = 'https://api.devnet.solana.com';
const USDC_DEVNET = '4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU';
const TOKEN_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';
const WANTED = {
    0: { instruction: 'initSubscriptionAuthority', mintPosition: 2, authorityPosition: 1, authorityRequired: true },
    1: { instruction: 'createFixedDelegation', authorityPosition: 1 },
    2: { instruction: 'createRecurringDelegation', authorityPosition: 1 },
    3: { instruction: 'revokeDelegation' },
    5: { instruction: 'transferRecurring', mintPosition: 4 },
    11: { instruction: 'subscribe', authorityPosition: 4, authorityRequired: true, planPosition: 2 },
    14: { instruction: 'revokeSubscriptionAuthority', mintPosition: 2 },
};
const TRANSACTION_CONFIG = { commitment: 'finalized', encoding: 'json', maxSupportedTransactionVersion: 0 };
const ACCOUNT_CONFIG = { commitment: 'finalized', encoding: 'base64' };
const AUTHORITY_LEN = 106;
const AUTHORITY_MINT_OFFSET = 33;
const PLAN_MINT_OFFSET = 43;
const MINT_LEN = 82;
const MINT_DECIMALS_OFFSET = 44;

if (discover) {
    await discoverExamples();
} else if (update) {
    await updateSnapshots();
} else {
    await verifySnapshots();
}

async function discoverExamples() {
    const found = new Map();
    let before;
    let scanned = 0;
    while (scanned < maxTransactions && found.size < Object.keys(WANTED).length) {
        const page = await rpc(ENDPOINT, 'getSignaturesForAddress', [
            PROGRAM_ID,
            { limit: 1000, commitment: 'finalized', ...(before ? { before } : {}) },
        ]);
        if (page.length === 0) break;
        for (const entry of page) {
            scanned += 1;
            before = entry.signature;
            if (entry.err) continue;
            if (scanned > maxTransactions || found.size === Object.keys(WANTED).length) break;
            await pause(400);
            const result = await rpc(ENDPOINT, 'getTransaction', [entry.signature, TRANSACTION_CONFIG]);
            if (!result || result.meta?.err) continue;
            const targets = wantedInstructions(result);
            if (targets.length !== 1) continue;
            const [{ index, opcode }] = targets;
            if (found.has(opcode)) continue;
            const snapshotAccountPositions = await usableSnapshotPositions(result, index, opcode);
            if (snapshotAccountPositions === null) continue;
            found.set(opcode, {
                id: WANTED[opcode].instruction.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`),
                instruction: WANTED[opcode].instruction,
                signature: entry.signature,
                slot: result.slot,
                version: transactionVersion(result),
                instructionIndex: index,
                opcode,
                snapshotAccountPositions,
                blockTime: result.blockTime,
            });
            process.stderr.write(`found ${WANTED[opcode].instruction} at ${entry.signature}\n`);
        }
    }
    const missing = Object.keys(WANTED).map(Number).filter(opcode => !found.has(opcode));
    process.stdout.write(`${JSON.stringify([...found.values()], null, 2)}\n`);
    process.stderr.write(`scanned ${scanned} signatures; missing opcodes: ${missing.join(', ') || 'none'}\n`);
}

/**
 * Whether a candidate is worth keeping, and which of its accounts can still be
 * snapshotted. An explicit mint account must be devnet USDC. Program accounts
 * we would snapshot (authority, plan) must still exist, be program-owned and
 * point at devnet USDC; for the delegation instructions the authority may
 * already be closed (nothing in their display depends on it), in which case
 * no snapshot position is recorded.
 */
async function usableSnapshotPositions(result, index, opcode) {
    const spec = WANTED[opcode];
    const accounts = instructionAccounts(result, index);
    if (spec.mintPosition !== undefined && accounts[spec.mintPosition] !== USDC_DEVNET) return null;
    const positions = [];
    if (spec.mintPosition !== undefined) positions.push(spec.mintPosition);
    for (const [position, offset, expectedLen, required] of [
        [spec.authorityPosition, AUTHORITY_MINT_OFFSET, AUTHORITY_LEN, spec.authorityRequired ?? false],
        [spec.planPosition, PLAN_MINT_OFFSET, undefined, true],
    ]) {
        if (position === undefined) continue;
        await pause(400);
        const response = await rpcWithContext(ENDPOINT, 'getAccountInfo', [accounts[position], ACCOUNT_CONFIG]);
        const account = response.value;
        if (!account) {
            if (required) return null;
            continue;
        }
        if (account.owner !== PROGRAM_ID) return null;
        const data = Buffer.from(account.data[0], 'base64');
        if (expectedLen !== undefined && data.length !== expectedLen) return null;
        if (encodeBase58(data.subarray(offset, offset + 32)) !== USDC_DEVNET) return null;
        positions.push(position);
    }
    return positions;
}

async function updateSnapshots() {
    const catalog = readJson(catalogPath);
    assertCatalogShape(catalog);
    const capturedAt = new Date().toISOString();
    const accounts = {};
    const transactions = {};
    const results = {};
    await mkdir(transactionsDir, { recursive: true });

    const snapshot = async address => {
        if (accounts[address]) return;
        const response = await rpcWithContext(ENDPOINT, 'getAccountInfo', [address, ACCOUNT_CONFIG]);
        const account = response.value;
        assert.ok(account, `Account ${address} does not exist`);
        const dataBase64 = account.data[0];
        const data = Buffer.from(dataBase64, 'base64');
        accounts[address] = {
            contextSlot: response.context.slot,
            dataBase64,
            dataSha256: sha256(data),
            owner: account.owner,
            space: data.length,
        };
    };

    for (const example of catalog.examples) {
        const result = await rpc(ENDPOINT, 'getTransaction', [example.signature, TRANSACTION_CONFIG]);
        validateTransaction(catalog, example, result);
        results[example.id] = result;
        const canonical = stringify(result);
        await writeFile(join(transactionsDir, example.transactionFile), canonical);
        transactions[example.id] = {
            blockTime: result.blockTime,
            resultSha256: sha256(canonical),
            slot: result.slot,
        };
        const metas = instructionAccounts(result, example.instructionIndex);
        for (const position of example.snapshotAccountPositions ?? []) {
            await snapshot(metas[position]);
        }
    }
    for (const address of catalog.extraSnapshotAddresses ?? []) {
        await snapshot(address);
    }

    await writeFile(accountsPath, stringify({ accounts }));
    await writeFile(provenancePath, stringify({
        capturedAt,
        rpc: {
            provider: 'Solana public devnet',
            endpoint: ENDPOINT,
            discoveryMethod: 'getSignaturesForAddress',
            transactionMethod: 'getTransaction',
            transactionConfig: TRANSACTION_CONFIG,
            accountMethod: 'getAccountInfo',
            accountConfig: ACCOUNT_CONFIG,
        },
        transactions,
    }));
    const cases = readJson(casesPath);
    cases.scenarios = [
        ...cases.scenarios.filter(scenario => !scenario.name.endsWith('-devnet')),
        ...devnetScenarios(catalog, results, accounts),
    ];
    await writeFile(casesPath, `${JSON.stringify(cases, null, 2)}\n`);
    process.stdout.write('updated devnet examples\n');
}

async function verifySnapshots() {
    const catalog = readJson(catalogPath);
    assertCatalogShape(catalog);
    const accountsDocument = readJson(accountsPath);
    const provenance = readJson(provenancePath);
    assert.ok(!Number.isNaN(Date.parse(provenance.capturedAt)), 'Invalid capture timestamp');
    assert.equal(provenance.rpc.provider, 'Solana public devnet');
    assert.equal(provenance.rpc.endpoint, ENDPOINT);
    assert.deepStrictEqual(provenance.rpc.transactionConfig, TRANSACTION_CONFIG);
    assert.deepStrictEqual(provenance.rpc.accountConfig, ACCOUNT_CONFIG);

    const results = {};
    for (const example of catalog.examples) {
        const raw = await readFile(join(transactionsDir, example.transactionFile), 'utf8');
        const result = JSON.parse(raw);
        results[example.id] = result;
        validateTransaction(catalog, example, result);
        assert.equal(provenance.transactions[example.id].slot, result.slot);
        assert.equal(provenance.transactions[example.id].blockTime, result.blockTime);
        assert.equal(
            sha256(stringify(result)),
            provenance.transactions[example.id].resultSha256,
            `Transaction hash changed for ${example.id}`,
        );
        const metas = instructionAccounts(result, example.instructionIndex);
        if (example.instruction === 'createFixedDelegation') {
            assert.notEqual(metas[3], '11111111111111111111111111111111', 'Demo spender must be usable');
            const signers = completeAccountKeys(result).slice(0, result.transaction.message.header.numRequiredSignatures);
            assert.ok(signers.includes(metas[3]), 'The selected demo spender signed as rent sponsor');
        }
        for (const position of example.snapshotAccountPositions ?? []) {
            const address = metas[position];
            const account = accountsDocument.accounts[address];
            assert.ok(account, `Missing snapshot ${address} for ${example.id}`);
            assert.ok(account.contextSlot >= example.slot, `Invalid snapshot slot for ${address}`);
            verifySnapshotBytes(address, account, example.expectedMint ?? USDC_DEVNET);
        }
    }
    for (const address of catalog.extraSnapshotAddresses ?? []) {
        const account = accountsDocument.accounts[address];
        assert.ok(account, `Missing extra snapshot ${address}`);
        verifySnapshotBytes(address, account);
    }

    const cases = readJson(casesPath);
    assert.deepStrictEqual(
        cases.scenarios.filter(scenario => scenario.name.endsWith('-devnet')),
        devnetScenarios(catalog, results, accountsDocument.accounts),
        'the -devnet scenarios are not derivable from the catalog',
    );

    const idlBytes = await readFile(idlPath);
    const idlProvenance = readJson(idlProvenancePath);
    assert.equal(sha256(idlBytes), idlProvenance.rootSha256, 'RootNode hash changed');
    assert.equal(sha256(await readFile(bundledIdlPath)), idlProvenance.rootSha256, 'bundled IDL copy drifted');
    const manifestEntry = readJson(manifestPath).idls.find(entry => entry.programId === PROGRAM_ID);
    assert.ok(manifestEntry, 'manifest has no Subscriptions entry');
    assert.equal(manifestEntry.sha256, idlProvenance.rootSha256, 'manifest digest drifted');
    const registry = readJson(registryPath);
    assert.equal(registry.cluster, 'devnet');
    const usdc = registry.tokens.find(token => token.mint === USDC_DEVNET);
    assert.ok(usdc, 'devnet registry has no USDC entry');
    const mintSnapshot = accountsDocument.accounts[USDC_DEVNET];
    if (mintSnapshot) {
        const data = Buffer.from(mintSnapshot.dataBase64, 'base64');
        assert.equal(data[MINT_DECIMALS_OFFSET], usdc.decimals, 'registry decimals disagree with the mint');
    }
    process.stdout.write('verified devnet examples\n');
}

function verifySnapshotBytes(address, account, expectedMint = USDC_DEVNET) {
    const bytes = Buffer.from(account.dataBase64, 'base64');
    assert.equal(bytes.length, account.space, `Snapshot size mismatch for ${address}`);
    assert.equal(sha256(bytes), account.dataSha256, `Snapshot hash mismatch for ${address}`);
    if (address === USDC_DEVNET) {
        assert.equal(account.owner, TOKEN_PROGRAM);
        assert.equal(bytes.length, MINT_LEN);
        return;
    }
    assert.equal(account.owner, PROGRAM_ID, `Snapshot ${address} is not program-owned`);
    if (bytes.length === AUTHORITY_LEN && bytes[0] === 0) {
        assert.equal(encodeBase58(bytes.subarray(AUTHORITY_MINT_OFFSET, AUTHORITY_MINT_OFFSET + 32)), expectedMint);
    } else {
        assert.equal(bytes[0], 1, `Snapshot ${address} is neither an authority nor a plan`);
        assert.equal(encodeBase58(bytes.subarray(PLAN_MINT_OFFSET, PLAN_MINT_OFFSET + 32)), expectedMint);
    }
}

function devnetScenarios(catalog, results, accounts) {
    return catalog.examples.map(example => {
        const result = results[example.id];
        const instruction = result.transaction.message.instructions[example.instructionIndex];
        const metas = instructionAccounts(result, example.instructionIndex);
        const accountData = {};
        for (const position of example.snapshotAccountPositions ?? []) {
            const address = metas[position];
            const account = accounts[address];
            accountData[address] = { programAddress: account.owner, dataBase64: account.dataBase64 };
        }
        const scenario = {
            name: `${example.instruction}-devnet`,
            dataBase64: Buffer.from(decodeBase58(instruction.data)).toString('base64'),
            accounts: instructionAccountsWithRoles(result, example.instructionIndex),
        };
        if (Object.keys(accountData).length > 0) {
            scenario.fetchAccounts = true;
            scenario.accountData = accountData;
        }
        return scenario;
    });
}

function validateTransaction(catalog, example, result) {
    assert.ok(result, `Missing transaction ${example.signature}`);
    assert.equal(result.slot, example.slot, `Unexpected slot for ${example.id}`);
    assert.equal(result.meta?.err, null, `Transaction failed for ${example.id}`);
    assert.equal(result.transaction.signatures[0], example.signature, `Signature mismatch for ${example.id}`);
    assert.equal(transactionVersion(result), example.version, `Version mismatch for ${example.id}`);
    const targets = wantedInstructions(result);
    assert.equal(targets.length, 1, `Expected one wanted instruction for ${example.id}`);
    assert.equal(targets[0].index, example.instructionIndex, `Instruction index mismatch for ${example.id}`);
    assert.equal(targets[0].opcode, example.opcode, `Opcode mismatch for ${example.id}`);
    assert.equal(WANTED[example.opcode].instruction, example.instruction, `Instruction name mismatch for ${example.id}`);
    assert.equal(catalog.programId, PROGRAM_ID);
}

function wantedInstructions(result) {
    const keys = completeAccountKeys(result);
    return result.transaction.message.instructions.flatMap((instruction, index) => {
        if (keys[instruction.programIdIndex] !== PROGRAM_ID) return [];
        const opcode = decodeBase58(instruction.data)[0];
        return opcode in WANTED ? [{ index, opcode }] : [];
    });
}

function assertCatalogShape(catalog) {
    assert.equal(catalog.schemaVersion, 2);
    assert.equal(catalog.cluster, 'devnet');
    assert.equal(catalog.programId, PROGRAM_ID);
    assert.deepStrictEqual(catalog.rpc, { kind: 'public', endpoint: ENDPOINT });
    assert.equal(catalog.tokenRegistryFile, 'token-registry-devnet.json');
    assert.ok(catalog.examples.length > 0, 'Devnet example catalog is empty');
}

function pause(milliseconds) {
    return new Promise(resolve => setTimeout(resolve, milliseconds));
}

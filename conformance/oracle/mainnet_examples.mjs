import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
    completeAccountKeys,
    decodeBase58,
    instructionAccounts as instructionAccountsAt,
    instructionAccountsWithRoles as instructionAccountsWithRolesAt,
    rpc,
    rpcWithContext,
    sha256,
    stringify,
} from './lib/rpc_snapshots.mjs';

const UPDATE_FLAG = '--update';
const args = process.argv.slice(2);
if (args.some(argument => argument !== UPDATE_FLAG)) {
    throw new Error(`Usage: node mainnet_examples.mjs [${UPDATE_FLAG}]`);
}

const update = args.includes(UPDATE_FLAG);
const oracleDir = dirname(fileURLToPath(import.meta.url));
const resourcesDir = join(oracleDir, '..', 'captures', 'mainnet');
const transactionsDir = join(resourcesDir, 'Transactions');
const conformanceDir = join(oracleDir, '..', 'fixtures', 'spl-token-instructions');
const catalog = JSON.parse(await readFile(join(resourcesDir, 'catalog.json'), 'utf8'));
const accountsPath = join(resourcesDir, 'accounts.json');
const provenancePath = join(resourcesDir, 'provenance.json');
const idlPath = join(conformanceDir, 'root.json');
const idlProvenancePath = join(conformanceDir, 'provenance.json');

if (update) {
    await updateSnapshots();
} else {
    await verifySnapshots();
}

async function updateSnapshots() {
    const apiKey = process.env.ALCHEMY_API_KEY?.trim();
    if (!apiKey) throw new Error('ALCHEMY_API_KEY is required with --update');
    const endpoint = `https://solana-mainnet.g.alchemy.com/v2/${apiKey}`;
    const capturedAt = new Date().toISOString();
    const accounts = {};
    const transactions = {};
    const results = {};
    await mkdir(transactionsDir, { recursive: true });

    for (const example of catalog.examples) {
        const result = await rpc(endpoint, 'getTransaction', [
            example.signature,
            { commitment: 'finalized', encoding: 'json', maxSupportedTransactionVersion: 0 },
        ]);
        validateTransaction(example, result);
        results[example.id] = result;
        const canonical = stringify(result);
        await writeFile(join(transactionsDir, example.transactionFile), canonical);
        transactions[example.id] = {
            blockTime: result.blockTime,
            resultSha256: sha256(canonical),
            slot: result.slot,
        };

        for (const position of example.snapshotAccountPositions ?? []) {
            const mint = instructionAccounts(result, example)[position];
            if (!accounts[mint]) {
                const response = await rpcWithContext(endpoint, 'getAccountInfo', [
                    mint,
                    { commitment: 'finalized', encoding: 'base64' },
                ]);
                const account = response.value;
                assert.ok(account, `Mint ${mint} does not exist`);
                const dataBase64 = account.data[0];
                const data = Buffer.from(dataBase64, 'base64');
                assert.equal(account.owner, catalog.programId, `Unexpected owner for ${mint}`);
                assert.equal(data.length, 82, `Unexpected Mint size for ${mint}`);
                accounts[mint] = {
                    contextSlot: response.context.slot,
                    dataBase64,
                    dataSha256: sha256(data),
                    owner: account.owner,
                    space: data.length,
                };
            }
        }
    }

    await writeFile(accountsPath, stringify({ accounts }));
    await writeFile(join(conformanceDir, 'cases.json'), stringify(buildConformanceCases(results, accounts)));
    await writeFile(provenancePath, stringify({
        capturedAt,
        rpc: {
            provider: 'Alchemy Solana Mainnet',
            transactionMethod: 'getTransaction',
            transactionConfig: {
                commitment: 'finalized',
                encoding: 'json',
                maxSupportedTransactionVersion: 0,
            },
            accountMethod: 'getAccountInfo',
            accountConfig: { commitment: 'finalized', encoding: 'base64' },
        },
        transactions,
    }));
    process.stdout.write('updated mainnet examples\n');
}

async function verifySnapshots() {
    assert.equal(catalog.schemaVersion, 2);
    assert.equal(catalog.cluster, 'mainnet-beta');
    assert.equal(catalog.programId, 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
    assert.deepStrictEqual(catalog.rpc, { kind: 'alchemy' });
    assert.equal(catalog.tokenRegistryFile, 'token-registry-mainnet.json');
    assert.ok(catalog.examples.length > 0, 'Mainnet example catalog is empty');
    const accountsDocument = JSON.parse(await readFile(accountsPath, 'utf8'));
    const provenance = JSON.parse(await readFile(provenancePath, 'utf8'));
    const idlProvenance = JSON.parse(await readFile(idlProvenancePath, 'utf8'));
    const idlBytes = await readFile(idlPath);
    const results = {};
    assert.equal(
        idlProvenance.idl.commit,
        '34a299b3ee0a9621f9f22b9b1b3f4f2893ef6bfe',
    );
    assert.equal(sha256(idlBytes), idlProvenance.rootSha256, 'RootNode hash changed');
    assert.ok(!Number.isNaN(Date.parse(provenance.capturedAt)), 'Invalid capture timestamp');
    assert.equal(provenance.rpc.provider, 'Alchemy Solana Mainnet');
    assert.deepStrictEqual(provenance.rpc.transactionConfig, {
        commitment: 'finalized',
        encoding: 'json',
        maxSupportedTransactionVersion: 0,
    });
    assert.deepStrictEqual(provenance.rpc.accountConfig, {
        commitment: 'finalized',
        encoding: 'base64',
    });

    for (const example of catalog.examples) {
        const raw = await readFile(join(transactionsDir, example.transactionFile), 'utf8');
        const result = JSON.parse(raw);
        results[example.id] = result;
        validateTransaction(example, result);
        assert.equal(provenance.transactions[example.id].slot, result.slot);
        assert.equal(provenance.transactions[example.id].blockTime, result.blockTime);
        assert.equal(
            sha256(stringify(result)),
            provenance.transactions[example.id].resultSha256,
            `Transaction hash changed for ${example.id}`,
        );
        for (const position of example.snapshotAccountPositions ?? []) {
            const mint = instructionAccounts(result, example)[position];
            const account = accountsDocument.accounts[mint];
            assert.ok(account, `Missing Mint snapshot ${mint}`);
            const bytes = Buffer.from(account.dataBase64, 'base64');
            assert.equal(account.owner, catalog.programId);
            assert.ok(account.contextSlot >= example.slot, `Invalid account context slot for ${mint}`);
            assert.equal(bytes.length, account.space);
            assert.equal(bytes.length, 82);
            assert.equal(sha256(bytes), account.dataSha256);
        }
    }
    const cases = JSON.parse(await readFile(join(conformanceDir, 'cases.json'), 'utf8'));
    assert.deepStrictEqual(cases, buildConformanceCases(results, accountsDocument.accounts));
    process.stdout.write('verified mainnet examples\n');
}

function buildConformanceCases(results, accounts) {
    const scenarios = [];
    for (const example of catalog.examples) {
        const result = results[example.id];
        const instruction = result.transaction.message.instructions[example.instructionIndex];
        const input = {
            accounts: instructionAccountsWithRoles(result, example),
            dataBase64: Buffer.from(decodeBase58(instruction.data)).toString('base64'),
        };
        scenarios.push({
            name: `${example.instruction}-online`,
            ...input,
            fetchAccounts: (example.snapshotAccountPositions ?? []).length > 0,
        });
        if ((example.snapshotAccountPositions ?? []).length > 0) {
            scenarios.push({
                name: `${example.instruction}-offline`,
                ...input,
                fetchAccounts: false,
            });
        }
    }
    const first = scenarios[0];
    return {
        programAddress: catalog.programId,
        dataBase64: first.dataBase64,
        accounts: first.accounts,
        accountData: Object.fromEntries(Object.entries(accounts).map(([address, account]) => [
            address,
            { dataBase64: account.dataBase64, programAddress: account.owner },
        ])),
        scenarios,
    };
}

function validateTransaction(example, result) {
    assert.ok(result, `Missing transaction ${example.signature}`);
    assert.equal(result.slot, example.slot, `Unexpected slot for ${example.id}`);
    assert.equal(result.meta?.err, null, `Transaction failed for ${example.id}`);
    assert.equal(result.transaction.signatures[0], example.signature, `Signature mismatch for ${example.id}`);
    const version = result.version === 0 ? 'v0' : result.version;
    assert.equal(version, example.version, `Version mismatch for ${example.id}`);
    const instructions = result.transaction.message.instructions;
    const target = instructions[example.instructionIndex];
    assert.ok(target, `Missing target instruction for ${example.id}`);
    const keys = completeAccountKeys(result);
    assert.equal(keys[target.programIdIndex], catalog.programId, `Program mismatch for ${example.id}`);
    const data = decodeBase58(target.data);
    assert.equal(data[0], example.opcode, `Opcode mismatch for ${example.id}`);
    const supportedCount = instructions.filter(instruction => {
        if (keys[instruction.programIdIndex] !== catalog.programId) return false;
        const bytes = decodeBase58(instruction.data);
        return [5, 9, 12, 13, 14, 15].includes(bytes[0]);
    }).length;
    assert.equal(supportedCount, 1, `Expected one supported target for ${example.id}`);
}

function instructionAccounts(result, example) {
    return instructionAccountsAt(result, example.instructionIndex);
}

function instructionAccountsWithRoles(result, example) {
    return instructionAccountsWithRolesAt(result, example.instructionIndex);
}

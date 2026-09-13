import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { getInstructionDisplay } from '@codama/dynamic-instructions';
import { AccountRole } from '@solana/instructions';
import { createFromJson } from 'codama';

const UPDATE_FLAG = '--update';
const args = process.argv.slice(2);
if (args.some(arg => arg !== UPDATE_FLAG)) {
    throw new Error(`Usage: node oracle.mjs [${UPDATE_FLAG}]`);
}
const update = args.includes(UPDATE_FLAG);

const roles = {
    readonly: AccountRole.READONLY,
    readonlySigner: AccountRole.READONLY_SIGNER,
    writable: AccountRole.WRITABLE,
    writableSigner: AccountRole.WRITABLE_SIGNER,
};

const oracleDir = dirname(fileURLToPath(import.meta.url));
const fixturesDir = join(oracleDir, '..', 'fixtures');
const fixtureNames = (await readdir(fixturesDir, { withFileTypes: true }))
    .filter(entry => entry.isDirectory())
    .map(entry => entry.name)
    .sort();

for (const fixtureName of fixtureNames) {
    const fixtureDir = join(fixturesDir, fixtureName);
    const rootJson = await readFile(join(fixtureDir, 'root.json'), 'utf8');
    const root = createFromJson(rootJson).getRoot();
    const fixture = JSON.parse(await readFile(join(fixtureDir, 'cases.json'), 'utf8'));
    await verifyProvenance(fixtureDir, fixture);
    const actual = {};

    for (const scenario of fixture.scenarios) {
        const scenarioAccounts = scenario.accounts ?? fixture.accounts;
        const scenarioAccountData = scenario.accountData ?? fixture.accountData;
        const instruction = {
            programAddress: scenario.programAddress ?? fixture.programAddress,
            data: Uint8Array.from(Buffer.from(scenario.dataBase64 ?? fixture.dataBase64, 'base64')),
            accounts: scenarioAccounts.map(account => {
                const role = roles[account.role];
                if (role === undefined) throw new Error(`Unknown account role: ${account.role}`);
                return { address: account.address, role };
            }),
        };

        const options = scenario.fetchAccounts
            ? {
                  fetchAccount: async address => {
                      const account = scenarioAccountData?.[address];
                      if (!account) return { address, exists: false };
                      const data = Uint8Array.from(Buffer.from(account.dataBase64, 'base64'));
                      return {
                          address,
                          data,
                          executable: false,
                          exists: true,
                          lamports: 0n,
                          programAddress: account.programAddress,
                          space: BigInt(data.length),
                      };
                  },
              }
            : {};

        let outcome;
        let threw = false;
        try {
            outcome = (await getInstructionDisplay(root, instruction, options)) ?? null;
        } catch (error) {
            if (!scenario.expectedError) throw error;
            threw = true;
            outcome = { error: scenario.expectedError };
        }
        if (scenario.expectedError && !threw) {
            throw new Error(`Expected ${scenario.expectedError} for ${fixtureName}/${scenario.name}`);
        }
        actual[scenario.name] = outcome;
    }

    const expectedPath = join(fixtureDir, 'expected.json');
    if (update) {
        await writeFile(expectedPath, `${JSON.stringify(actual, null, 2)}\n`);
        process.stdout.write(`updated ${fixtureName}\n`);
    } else {
        const expected = JSON.parse(await readFile(expectedPath, 'utf8'));
        assert.deepStrictEqual(actual, expected, `Codama output changed for fixture ${fixtureName}`);
        process.stdout.write(`verified ${fixtureName}\n`);
    }
}

async function verifyProvenance(fixtureDir, fixture) {
    let provenance;
    try {
        provenance = JSON.parse(await readFile(join(fixtureDir, 'provenance.json'), 'utf8'));
    } catch (error) {
        if (error?.code === 'ENOENT') return;
        throw error;
    }

    const snapshot = provenance.accountSnapshot;
    if (!snapshot) return;
    const account = fixture.accountData?.[snapshot.address];
    assert.ok(account, `Missing provenance account ${snapshot.address}`);
    const data = Buffer.from(account.dataBase64, 'base64');
    assert.equal(data.length, snapshot.space, `Snapshot size changed for ${snapshot.address}`);
    assert.equal(account.programAddress, snapshot.owner, `Snapshot owner changed for ${snapshot.address}`);
    assert.equal(
        createHash('sha256').update(data).digest('hex'),
        snapshot.dataSha256,
        `Snapshot data changed for ${snapshot.address}`,
    );
}

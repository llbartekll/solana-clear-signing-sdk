// Derives the display-decorated Subscriptions RootNode from the pristine
// program IDL. Everything here is reproducible: the same pristine bytes, the
// same display spec, and the same pinned Codama produce byte-identical output.
//
//   node author_subscriptions.mjs          write the IDL, provenance, bundled copy and manifest digest
//   node author_subscriptions.mjs --check  regenerate in memory and fail on any drift
//
// Wire layouts, discriminators, accounts, PDAs, errors and default values are
// the pristine IDL's. The only transformations are two standard Codama
// visitors (inline the argument structs, lift their fields to top-level
// arguments so `${data.<field>}` becomes addressable) and the display
// metadata declared in `display.json`. The derived root also links a pinned
// SPL Mint account layout in an additional program with no instructions.

import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
    createFromJson,
    flattenInstructionDataArgumentsVisitor,
    getValidationItemsVisitor,
    throwValidatorItemsVisitor,
    unwrapDefinedTypesVisitor,
} from 'codama';

import { readJson, sha256, stringify } from './lib/json.mjs';

const CHECK_FLAG = '--check';
const args = process.argv.slice(2);
if (args.some(arg => arg !== CHECK_FLAG)) {
    throw new Error(`Usage: node author_subscriptions.mjs [${CHECK_FLAG}]`);
}
const check = args.includes(CHECK_FLAG);

const oracleDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(oracleDir, '..', '..');
const paths = {
    pristine: join(repoRoot, 'vendor', 'idl', 'subscriptions.json'),
    vendorFixture: join(repoRoot, 'vendor', 'fixtures', 'recurring_delegation_devnet.json'),
    spec: join(repoRoot, 'conformance', 'fixtures', 'subscriptions', 'display.json'),
    tokenIdl: join(repoRoot, 'conformance', 'fixtures', 'spl-token-instructions', 'root.json'),
    root: join(repoRoot, 'conformance', 'fixtures', 'subscriptions', 'root.json'),
    provenance: join(repoRoot, 'conformance', 'fixtures', 'subscriptions', 'provenance.json'),
    bundledCopy: join(repoRoot, 'ios-demo', 'Resources', 'Srf39', 'subscriptions-root.json'),
    manifest: join(repoRoot, 'ios-demo', 'Resources', 'Srf39', 'srf39-manifest.json'),
    codamaPackage: join(oracleDir, 'node_modules', 'codama', 'package.json'),
};

const PROGRAM_ID = 'De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44';
const PRISTINE_SHA256 = 'ad90a04709e3c314d3bcd2516f1b85f4f98ae76e294a912fc12f72b8b4823e80';
const TOKEN_IDL_SHA256 = '472f41c79165064ba7bd7cd8623d0b730a227b6bfc12faca77dc8e4702b7bdc1';
const TRANSFER_INSTRUCTIONS = ['transferFixed', 'transferRecurring', 'transferSubscription'];
const UPSTREAM = {
    repository: 'https://github.com/solana-program/subscriptions',
    commit: '594316da4fd2b7d998f2c2086b3cc6741aba0508',
    path: 'idl/subscriptions.json',
};
const UNWRAPPED_TYPES = [
    'createFixedDelegationData',
    'createRecurringDelegationData',
    'transferData',
    'planData',
    'updatePlanData',
    'subscribeData',
];
const EXPECTED_ARGUMENTS = {
    createFixedDelegation: ['discriminator', 'nonce', 'amount', 'expiryTs', 'expectedSubscriptionAuthorityInitId'],
    createRecurringDelegation: [
        'discriminator',
        'nonce',
        'amountPerPeriod',
        'periodLengthS',
        'startTs',
        'expiryTs',
        'expectedSubscriptionAuthorityInitId',
    ],
    subscribe: [
        'discriminator',
        'planId',
        'planBump',
        'expectedMint',
        'expectedAmount',
        'expectedPeriodHours',
        'expectedCreatedAt',
        'expectedSubscriptionAuthorityInitId',
    ],
    transferRecurring: ['discriminator', 'amount', 'delegator', 'mint'],
    createPlan: ['discriminator', 'planId', 'mint', 'terms', 'endTs', 'destinations', 'pullers', 'metadataUri'],
};
const PLACEHOLDER_PATTERN = /\$\{\s*(data|accounts)\.([a-zA-Z0-9_]+)\s*\}/g;

const pristineBytes = readFileSync(paths.pristine);
assert.equal(sha256(pristineBytes), PRISTINE_SHA256, 'vendor/idl/subscriptions.json changed; re-pin the source');
const specBytes = readFileSync(paths.spec);
const spec = JSON.parse(specBytes.toString('utf8'));
assert.equal(spec.schemaVersion, 1);
assert.equal(spec.programId, PROGRAM_ID);

const codama = createFromJson(pristineBytes.toString('utf8'));
assert.equal(codama.getRoot().version, '1.0.0', 'pristine RootNode version');
codama.update(unwrapDefinedTypesVisitor(UNWRAPPED_TYPES));
codama.update(flattenInstructionDataArgumentsVisitor());
const decorated = decorate(structuredClone(codama.getRoot()), spec);
const tokenIdlBytes = readFileSync(paths.tokenIdl);
assert.equal(sha256(tokenIdlBytes), TOKEN_IDL_SHA256, 'SPL Mint layout source changed; review and re-pin it');
const tokenProgram = JSON.parse(tokenIdlBytes.toString('utf8')).program;
const mintAccount = tokenProgram.accounts.find(account => account.name === 'mint');
assert.ok(mintAccount, 'SPL Token IDL must contain the full Mint layout');
assert.equal(decorated.additionalPrograms?.length ?? 0, 0);
decorated.additionalPrograms = [{
    kind: 'programNode',
    name: tokenProgram.name,
    publicKey: tokenProgram.publicKey,
    accounts: [structuredClone(mintAccount)],
    instructions: [],
}];
assertStructure(decorated);
createFromJson(JSON.stringify(decorated)).accept(throwValidatorItemsVisitor(getValidationItemsVisitor(), 'error'));

const rootJson = stringify(decorated);
const rootSha256 = sha256(rootJson);
const vendorFixture = readJson(paths.vendorFixture);
const authorityBytes = Buffer.from(vendorFixture.authorityAccountDataB64, 'base64');
const provenance = {
    idl: {
        ...UPSTREAM,
        vendoredCopy: 'vendor/idl/subscriptions.json',
        sourceSha256: PRISTINE_SHA256,
        programVersion: decorated.program.version,
        generatedBy: 'program build.rs (#[codama] attributes)',
    },
    derivation: {
        tool: 'conformance/oracle/author_subscriptions.mjs',
        codama: readJson(paths.codamaPackage).version,
        visitors: [
            `unwrapDefinedTypesVisitor([${UNWRAPPED_TYPES.join(', ')}])`,
            'flattenInstructionDataArgumentsVisitor()',
            'decorate(display.json)',
        ],
        displaySpec: 'display.json',
        displaySpecSha256: sha256(specBytes),
        rootVersion: decorated.version,
        rootVersionNote:
            'Set by @codama/nodes CODAMA_VERSION when visitors rebuild the root; the pristine IDL says 1.0.0.',
    },
    accountSnapshot: {
        address: vendorFixture.accounts[1],
        owner: vendorFixture.authorityAccountOwner,
        space: authorityBytes.length,
        dataSha256: sha256(authorityBytes),
        capturedFrom: 'vendor/fixtures/recurring_delegation_devnet.json (devnet, 2026-07-03)',
    },
    linkedMintLayout: {
        source: 'conformance/fixtures/spl-token-instructions/root.json',
        sourceSha256: TOKEN_IDL_SHA256,
        programId: tokenProgram.publicKey,
        account: mintAccount.name,
    },
    rootSha256,
    notes: [
        'Wire layouts, discriminators, accounts, pdas, errors and defaultValues are the pristine IDL’s; standard display nodes and the namespaced x-solana-clearsign token amount binding were added.',
        'Transfer amounts resolve decimals from the instruction’s tokenMint account through a cross-program accountLink and providedNode. The additional Token program supplies only the full Mint account layout, with no Token instructions.',
        'Delegation approvals have no mint account; subscribe carries expectedMint as an argument. Their decimals remain unavailable to the pinned Codama display reference, so those amounts render as `<raw> (raw)`.',
        'The SDK additionally presents subscribe.expectedAmount using tokenMetadata(expectedMint).decimals; this extension does not change the pinned reference output.',
        'No accountLink is declared on subscriptionAuthority: nothing in the display could use its contents.',
    ],
};
const provenanceJson = stringify(provenance);

const manifest = readJson(paths.manifest);
const manifestEntries = manifest.idls.filter(entry => entry.programId === PROGRAM_ID);
assert.equal(manifestEntries.length, 1, 'manifest must contain exactly one Subscriptions entry');
assert.equal(manifestEntries[0].file, 'subscriptions-root.json', 'manifest must reference the bundled IDL');
manifestEntries[0].sha256 = rootSha256;

const outputs = [
    [paths.root, rootJson],
    [paths.bundledCopy, rootJson],
    [paths.provenance, provenanceJson],
    [paths.manifest, `${JSON.stringify(manifest, null, 2)}\n`],
];
if (check) {
    for (const [path, expected] of outputs) {
        let actual;
        try {
            actual = readFileSync(path, 'utf8');
        } catch (error) {
            throw new Error(`Missing ${path}: run author:subscriptions (${error.message})`);
        }
        assert.equal(actual, expected, `${path} is not the reproducible derivation`);
    }
    process.stdout.write(`verified subscriptions derivation (rootSha256 ${rootSha256})\n`);
} else {
    for (const [path, content] of outputs) {
        writeFileSync(path, content);
    }
    process.stdout.write(`wrote subscriptions derivation (rootSha256 ${rootSha256})\n`);
}

/** Applies `display.json` to a plain RootNode object. Fails loudly on any name it cannot find. */
function decorate(root, displaySpec) {
    const program = root.program;
    const instructions = new Map(program.instructions.map(instruction => [instruction.name, instruction]));
    for (const [instructionName, entry] of Object.entries(displaySpec.instructions)) {
        assert.ok(instructions.has(instructionName), `display.json names unknown instruction ${instructionName}`);
    }
    for (const instruction of program.instructions) {
        const entry = displaySpec.instructions[instruction.name] ?? {};
        const argumentSpecs = { ...displaySpec.defaults.arguments, ...(entry.arguments ?? {}) };
        const accountSpecs = { ...displaySpec.defaults.accounts, ...(entry.accounts ?? {}) };
        for (const name of Object.keys(entry.arguments ?? {})) {
            assert.ok(
                instruction.arguments.some(argument => argument.name === name),
                `display.json: ${instruction.name} has no argument ${name}`,
            );
        }
        for (const name of Object.keys(entry.accounts ?? {})) {
            assert.ok(
                instruction.accounts.some(account => account.name === name),
                `display.json: ${instruction.name} has no account ${name}`,
            );
        }
        for (const argument of instruction.arguments) {
            const argumentSpec = argumentSpecs[argument.name];
            if (!argumentSpec) continue;
            const display = { kind: 'structFieldDisplayNode' };
            if (argumentSpec.label !== undefined) display.label = argumentSpec.label;
            if (argumentSpec.skip !== undefined) display.skip = argumentSpec.skip;
            if (Object.keys(display).length > 1) argument.display = display;
            if (argumentSpec.number) {
                assert.equal(
                    argument.type.kind,
                    'numberTypeNode',
                    `display.json: ${instruction.name}.${argument.name} is not a number`,
                );
                argument.type.display = numberDisplay(argumentSpec.number);
            }
        }
        for (const account of instruction.accounts) {
            const accountSpec = accountSpecs[account.name];
            if (!accountSpec) continue;
            const display = { kind: 'instructionAccountDisplayNode' };
            if (accountSpec.label !== undefined) display.label = accountSpec.label;
            if (accountSpec.skip !== undefined) display.skip = accountSpec.skip;
            if (Object.keys(display).length > 1) account.display = display;
            if (accountSpec.accountLink) account.accountLink = structuredClone(accountSpec.accountLink);
        }
        if (entry.provides) instruction.provides = structuredClone(entry.provides);
        if (entry.intent !== undefined || entry.interpolatedIntent !== undefined || entry.tokenAmounts !== undefined) {
            const display = { kind: 'instructionDisplayNode' };
            if (entry.intent !== undefined) display.intent = entry.intent;
            if (entry.interpolatedIntent !== undefined) {
                for (const match of entry.interpolatedIntent.matchAll(PLACEHOLDER_PATTERN)) {
                    const [, scope, name] = match;
                    const members = scope === 'data' ? instruction.arguments : instruction.accounts;
                    assert.ok(
                        members.some(member => member.name === name),
                        `display.json: ${instruction.name} sentence references unknown ${scope}.${name}`,
                    );
                }
                display.interpolatedIntent = entry.interpolatedIntent;
            }
            if (entry.tokenAmounts !== undefined) {
                const amounts = new Set();
                for (const binding of entry.tokenAmounts) {
                    assert.ok(!amounts.has(binding.amount), 'duplicate token amount binding');
                    amounts.add(binding.amount);
                    const amount = instruction.arguments.find(arg => arg.name === binding.amount);
                    assert.ok(amount?.type.kind === 'numberTypeNode' && ['u64', 'i64'].includes(amount.type.format)
                        && amount.type.display?.kind === 'amountNumberDisplayNode', 'token binding needs an amount argument');
                    assert.ok(['argument', 'account'].includes(binding.mint.source), 'unknown token mint source');
                    const members = binding.mint.source === 'argument' ? instruction.arguments : instruction.accounts;
                    const mint = members.find(member => member.name === binding.mint.name);
                    assert.ok(mint && (binding.mint.source === 'account' || mint.type.kind === 'publicKeyTypeNode'),
                        'token binding needs a public-key argument or named account');
                }
                display['x-solana-clearsign'] = { tokenAmounts: structuredClone(entry.tokenAmounts) };
            }
            instruction.display = display;
        }
    }
    return root;
}

function numberDisplay(numberSpec) {
    switch (numberSpec.kind) {
        case 'amount': {
            const node = { kind: 'amountNumberDisplayNode' };
            if (numberSpec.decimals?.inject) {
                node.decimals = { kind: 'injectedValueNode', key: numberSpec.decimals.inject };
            }
            if (numberSpec.unit !== undefined) {
                node.unit = { kind: 'stringValueNode', string: numberSpec.unit };
            }
            return node;
        }
        case 'dateTime':
            return { kind: 'dateTimeNumberDisplayNode' };
        case 'duration':
            return { kind: 'durationNumberDisplayNode' };
        default:
            throw new Error(`display.json: unknown number display kind ${numberSpec.kind}`);
    }
}

/** Guards against silent drops by the identity visitor. */
function assertStructure(root) {
    assert.equal(root.kind, 'rootNode');
    assert.equal(root.standard, 'codama');
    assert.equal(root.version, '1.8.0', 'RootNode version after the pinned visitors');
    const program = root.program;
    assert.equal(program.publicKey, PROGRAM_ID);
    assert.equal(program.version, '0.1.0');
    assert.equal(program.instructions.length, 16);
    assert.equal(program.accounts.length, 6);
    assert.equal(program.errors.length, 75);
    assert.equal(program.pdas.length, 6);
    assert.deepEqual(
        program.definedTypes.map(definedType => definedType.name),
        ['planTerms', 'accountDiscriminator', 'planStatus', 'header'],
    );
    for (const [name, expected] of Object.entries(EXPECTED_ARGUMENTS)) {
        const instruction = program.instructions.find(candidate => candidate.name === name);
        assert.ok(instruction, `instruction ${name}`);
        assert.deepEqual(
            instruction.arguments.map(argument => argument.name),
            expected,
            `flattened arguments of ${name}`,
        );
    }
    for (const instruction of program.instructions) {
        const discriminator = instruction.arguments[0];
        assert.equal(discriminator.name, 'discriminator');
        assert.equal(discriminator.defaultValue.kind, 'numberValueNode');
        assert.equal(discriminator.display?.skip, 'always');
        if (TRANSFER_INSTRUCTIONS.includes(instruction.name)) {
            assert.deepEqual(instruction.provides, [{
                kind: 'providedNode', name: 'decimals',
                node: { kind: 'accountFieldValueNode', account: 'tokenMint', path: 'decimals' },
            }]);
            const mint = instruction.accounts.find(account => account.name === 'tokenMint');
            assert.deepEqual(mint.accountLink, {
                kind: 'accountLinkNode', name: 'mint', program: { kind: 'programLinkNode', name: 'token' },
            });
        } else {
            assert.equal(instruction.provides, undefined, 'no scale source is available for this instruction');
        }
    }
}

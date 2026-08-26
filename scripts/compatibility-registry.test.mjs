import assert from "node:assert/strict";
import { generateKeyPairSync, verify } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { canonicalPayload, payloadHash, readUnsignedBundle, signBundle } from "./compatibility-registry.mjs";
import { buildCompatibilityRegistrySite } from "./build-compatibility-registry-site.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));
const scratchParent = path.join(root, "scratch");
mkdirSync(scratchParent, { recursive: true });
const scratch = mkdtempSync(path.join(scratchParent, "compatibility-registry-test-"));
process.once("exit", () => rmSync(scratch, { recursive: true, force: true }));

function readSource(relativePath) {
  return JSON.parse(readFileSync(path.join(root, relativePath), "utf8"));
}

function assertUnsignedSource(bundle, runtime, collectionKey) {
  assert.equal(bundle.format, 1);
  assert.equal(bundle.runtime, runtime);
  assert.equal(bundle.sequence, 1);
  assert.equal(Object.hasOwn(bundle, "signature"), false);
  assert.equal(Object.hasOwn(bundle, "keyId"), false);
  assert.ok(Array.isArray(bundle[collectionKey]) && bundle[collectionKey].length > 0);
}

function latestSequencePath(runtimeDirectory) {
  const emittedSequences = readdirSync(runtimeDirectory)
    .filter((file) => /^\d+\.json$/.test(file))
    .sort((left, right) => Number(path.basename(left, ".json")) - Number(path.basename(right, ".json")))
    .map((file) => path.join(runtimeDirectory, file));
  if (!emittedSequences.length) throw new Error(`no emitted signed sequences found in ${runtimeDirectory}`);
  return emittedSequences.at(-1);
}

function assertNoPrivateKeyLeak(currentFile, privateKeyPem) {
  const hasPemHeader = /BEGIN PRIVATE KEY|END PRIVATE KEY/.test(currentFile);
  const hasPrivateTextEscaped = currentFile.replace(/\\n/g, "\n").includes(privateKeyPem);
  const hasPrivateTextUnescaped = currentFile.includes(privateKeyPem.replace(/\r?\n/g, "\\n"));
  const hasRawPrivateText = currentFile.includes(privateKeyPem);
  assert.equal(hasPemHeader, false);
  assert.equal(hasPrivateTextEscaped, false);
  assert.equal(hasPrivateTextUnescaped, false);
  assert.equal(hasRawPrivateText, false);
}

function writeJson(file, value, spaces = 2) {
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, `${JSON.stringify(value, null, spaces)}\n`);
}

function createSourceFixture(name, sources) {
  const fixtureRoot = path.join(scratch, name);
  for (const [directory, bundles] of Object.entries(sources)) {
    for (const bundle of bundles) {
      writeJson(path.join(fixtureRoot, "registry", "compatibility", directory, `${bundle.sequence}.json`), bundle);
    }
  }
  return fixtureRoot;
}

function stageHistory(output, histories) {
  const checkpoints = {};
  for (const [runtime, entries] of Object.entries(histories)) {
    for (const entry of entries) {
      mkdirSync(path.join(output, runtime), { recursive: true });
      writeFileSync(path.join(output, runtime, `${entry.bundle.sequence}.json`), entry.bytes);
    }
    const latest = entries.at(-1);
    checkpoints[runtime] = {
      sequence: latest.bundle.sequence,
      payloadHash: latest.checkpointHash ?? payloadHash(latest.bundle),
    };
  }
  writeJson(path.join(output, "checkpoints.json"), checkpoints);
  return checkpoints;
}

const opencode = readSource("registry/compatibility/opencode/1.json");
const grok = readSource("registry/compatibility/grok/1.json");
const openclaw = readSource("registry/compatibility/openclaw/1.json");

assertUnsignedSource(opencode, "opencode", "schemas");
assertUnsignedSource(grok, "grok-build", "schemas");
assertUnsignedSource(openclaw, "openclaw", "profiles");
assert.equal(
  payloadHash(openclaw),
  "c14d44e311bbf1f540836b6881403bd6147463bfaeebaaf756d4c6111dc0bb49",
  "OpenClaw genesis source must exactly match Cave's immutable built-in payload",
);
assert.equal(openclaw.profiles.length, 1);
assert.equal(
  openclaw.profiles[0].requires.agentEventSchemaHash,
  "ed68832d5ecbc52de7ad5394933cb2a0df7b0f0b6f7a880127f688becbd76bd2",
);
assert.deepEqual(openclaw.profiles[0].eventNames, ["agent", "session.tool"]);

assert.equal(
  canonicalPayload({ z: 1, runtime: "opencode", array: [true], signature: { algorithm: "ed25519", value: "excluded" } }),
  '{"array":[true],"runtime":"opencode","z":1}',
);
assert.match(payloadHash(opencode), /^[a-f0-9]{64}$/);

const { privateKey, publicKey } = generateKeyPairSync("ed25519");
const privateKeyPem = privateKey.export({ type: "pkcs8", format: "pem" }).toString();
const signedOpenCode = signBundle(opencode, privateKeyPem);
assert.equal(signedOpenCode.keyId, undefined);
assert.equal(signedOpenCode.signature.algorithm, "ed25519");
assert.equal(
  verify(null, Buffer.from(canonicalPayload(signedOpenCode)), publicKey, Buffer.from(signedOpenCode.signature.value, "base64")),
  true,
);
assert.throws(() => signBundle({ ...opencode, signature: { algorithm: "ed25519", value: "not-allowed" } }, privateKeyPem));
assert.throws(() => signBundle({ ...opencode, schemas: [] }, privateKeyPem));
assert.throws(() => signBundle({ ...opencode, unsupported: true }, privateKeyPem));
assert.throws(() => signBundle({ ...opencode, keyId: undefined }, privateKeyPem), /field keyId must not be undefined/);
assert.throws(() => signBundle(opencode, generateKeyPairSync("rsa", { modulusLength: 2048 }).privateKey.export({ type: "pkcs8", format: "pem" }).toString()));
assert.equal(signBundle({ ...opencode, sequence: 2, keyId: "opencode-2027-01" }, privateKeyPem).keyId, "opencode-2027-01");
const { privateKey: openclawPrivateKey, publicKey: openclawPublicKey } = generateKeyPairSync("ed25519");
const openclawPrivateKeyPem = openclawPrivateKey.export({ type: "pkcs8", format: "pem" }).toString();
const signedOpenClaw = signBundle(openclaw, openclawPrivateKeyPem);
assert.equal(signedOpenClaw.keyId, undefined);
assert.equal(signedOpenClaw.signature.algorithm, "ed25519");
assert.equal(
  verify(null, Buffer.from(canonicalPayload(signedOpenClaw)), openclawPublicKey, Buffer.from(signedOpenClaw.signature.value, "base64")),
  true,
);
assert.equal(signBundle({ ...openclaw, retiredProfileIds: ["openclaw-agent-tool-v0"] }, openclawPrivateKeyPem).signature.algorithm, "ed25519");
assert.throws(() => signBundle({ ...openclaw, schemas: opencode.schemas }, openclawPrivateKeyPem));
assert.throws(() => signBundle({ ...openclaw, retiredSchemaIds: ["opencode-run-json-v0"] }, openclawPrivateKeyPem));
assert.throws(() => signBundle({ ...openclaw, retiredProfileIds: ["openclaw-agent-tool-v0"], retiredSchemaIds: ["opencode-run-json-v0"] }, openclawPrivateKeyPem));
for (const { bundle, keyPem } of [
  { bundle: opencode, keyPem: privateKeyPem },
  { bundle: grok, keyPem: generateKeyPairSync("ed25519").privateKey.export({ type: "pkcs8", format: "pem" }).toString() },
]) {
  assert.throws(() => signBundle({ ...bundle, profiles: openclaw.profiles }, keyPem));
  assert.throws(() => signBundle({ ...bundle, retiredProfileIds: ["openclaw-agent-tool-v0"] }, keyPem));
}
assert.deepEqual(readUnsignedBundle(path.join(root, "registry/compatibility/grok/1.json")), grok);

const grokKey = generateKeyPairSync("ed25519").privateKey.export({ type: "pkcs8", format: "pem" }).toString();
const output = path.join(scratch, "fresh-site");
writeJson(path.join(output, "checkpoints.json"), {});
const checkpoints = buildCompatibilityRegistrySite({
  root,
  output,
  opencodePrivateKeyPem: privateKeyPem,
  grokPrivateKeyPem: grokKey,
  openclawPrivateKeyPem,
});
const opencodeCurrent = await readFile(path.join(output, "opencode/current.json"), "utf8");
const opencodeLatest = await readFile(latestSequencePath(path.join(output, "opencode")), "utf8");
const grokCurrent = await readFile(path.join(output, "grok/current.json"), "utf8");
const grokLatest = await readFile(latestSequencePath(path.join(output, "grok")), "utf8");
const openclawCurrent = await readFile(path.join(output, "openclaw/current.json"), "utf8");
const openclawLatest = await readFile(latestSequencePath(path.join(output, "openclaw")), "utf8");
assert.equal(opencodeCurrent, opencodeLatest);
assert.equal(grokCurrent, grokLatest);
assert.equal(openclawCurrent, openclawLatest);
assert.equal(JSON.parse(opencodeCurrent).keyId, undefined, "genesis payload must match Cave's pinned sequence-one contract");
assert.equal(JSON.parse(grokCurrent).keyId, undefined, "genesis payload must match Cave's pinned sequence-one contract");
const publishedOpenClaw = JSON.parse(openclawCurrent);
assert.equal(publishedOpenClaw.keyId, undefined, "OpenClaw genesis payload must match Cave's pinned sequence-one contract");
assert.equal(
  verify(null, Buffer.from(canonicalPayload(publishedOpenClaw)), openclawPublicKey, Buffer.from(publishedOpenClaw.signature.value, "base64")),
  true,
);
assert.equal(checkpoints.openclaw.sequence, 1);
assert.match(checkpoints.openclaw.payloadHash, /^[a-f0-9]{64}$/);
assert.equal(checkpoints.openclaw.payloadHash, payloadHash(publishedOpenClaw));
assert.deepEqual(JSON.parse(await readFile(path.join(output, "checkpoints.json"), "utf8")), checkpoints);
assertNoPrivateKeyLeak(opencodeCurrent, privateKeyPem);
assertNoPrivateKeyLeak(grokCurrent, grokKey);
assertNoPrivateKeyLeak(openclawCurrent, openclawPrivateKeyPem);

const wrongRuntimeRoot = createSourceFixture("wrong-runtime-source", {
  opencode: [{ ...openclaw }],
  grok: [grok],
  openclaw: [openclaw],
});
const wrongRuntimeOutput = path.join(scratch, "wrong-runtime-output");
writeJson(path.join(wrongRuntimeOutput, "checkpoints.json"), {});
assert.throws(
  () =>
    buildCompatibilityRegistrySite({
      root: wrongRuntimeRoot,
      output: wrongRuntimeOutput,
      opencodePrivateKeyPem: privateKeyPem,
      grokPrivateKeyPem: grokKey,
      openclawPrivateKeyPem,
    }),
  /source runtime mismatch.*opencode.*openclaw/i,
);

const { privateKey: oldOpenCodeKey, publicKey: oldOpenCodePublicKey } = generateKeyPairSync("ed25519");
const { privateKey: oldGrokKey } = generateKeyPairSync("ed25519");
const oldSignedOpenCode = signBundle(opencode, oldOpenCodeKey.export({ type: "pkcs8", format: "pem" }).toString());
const oldSignedGrok = signBundle(grok, oldGrokKey.export({ type: "pkcs8", format: "pem" }).toString());
const oldOpenCodeBytes = `${JSON.stringify(oldSignedOpenCode, null, 4)}\n`;
const oldGrokBytes = `${JSON.stringify(oldSignedGrok)}\n`;
const rotatedOutput = path.join(scratch, "rotated-site");
stageHistory(rotatedOutput, {
  opencode: [{ bundle: oldSignedOpenCode, bytes: oldOpenCodeBytes }],
  grok: [{ bundle: oldSignedGrok, bytes: oldGrokBytes }],
});
const rotatedCheckpoints = buildCompatibilityRegistrySite({
  root,
  output: rotatedOutput,
  opencodePrivateKeyPem: generateKeyPairSync("ed25519").privateKey.export({ type: "pkcs8", format: "pem" }).toString(),
  grokPrivateKeyPem: generateKeyPairSync("ed25519").privateKey.export({ type: "pkcs8", format: "pem" }).toString(),
  openclawPrivateKeyPem,
});
assert.equal(await readFile(path.join(rotatedOutput, "opencode/1.json"), "utf8"), oldOpenCodeBytes);
assert.equal(await readFile(path.join(rotatedOutput, "grok/1.json"), "utf8"), oldGrokBytes);
assert.equal(await readFile(path.join(rotatedOutput, "opencode/current.json"), "utf8"), oldOpenCodeBytes);
assert.equal(await readFile(path.join(rotatedOutput, "grok/current.json"), "utf8"), oldGrokBytes);
assert.equal(
  await readFile(path.join(rotatedOutput, "openclaw/current.json"), "utf8"),
  await readFile(path.join(rotatedOutput, "openclaw/1.json"), "utf8"),
);
assert.equal(rotatedCheckpoints.opencode.payloadHash, payloadHash(oldSignedOpenCode));
assert.equal(rotatedCheckpoints.grok.payloadHash, payloadHash(oldSignedGrok));
assert.equal(rotatedCheckpoints.openclaw.sequence, 1);

const successorOpenCode = {
  ...opencode,
  sequence: 2,
  issuedAt: "2027-01-01T00:00:00.000Z",
  expiresAt: "2028-01-01T00:00:00.000Z",
  keyId: "opencode-rotation-test",
};
const successorRoot = createSourceFixture("rotation-successor-source", {
  opencode: [opencode, successorOpenCode],
  grok: [grok],
  openclaw: [openclaw],
});
const successorOutput = path.join(scratch, "rotation-successor-site");
stageHistory(successorOutput, {
  opencode: [{ bundle: oldSignedOpenCode, bytes: oldOpenCodeBytes }],
});
const { privateKey: newOpenCodeKey, publicKey: newOpenCodePublicKey } =
  generateKeyPairSync("ed25519");
const newOpenCodeKeyPem = newOpenCodeKey.export({ type: "pkcs8", format: "pem" }).toString();
const successorCheckpoints = buildCompatibilityRegistrySite({
  root: successorRoot,
  output: successorOutput,
  opencodePrivateKeyPem: newOpenCodeKeyPem,
  grokPrivateKeyPem: grokKey,
  openclawPrivateKeyPem,
});
const successorSequenceOneBytes = await readFile(
  path.join(successorOutput, "opencode/1.json"),
  "utf8",
);
const successorSequenceTwoBytes = await readFile(
  path.join(successorOutput, "opencode/2.json"),
  "utf8",
);
const successorCurrentBytes = await readFile(
  path.join(successorOutput, "opencode/current.json"),
  "utf8",
);
const successorSequenceTwo = JSON.parse(successorSequenceTwoBytes);
assert.equal(successorSequenceOneBytes, oldOpenCodeBytes);
assert.equal(successorCurrentBytes, successorSequenceTwoBytes);
assert.equal(successorSequenceTwo.sequence, 2);
assert.equal(successorSequenceTwo.keyId, "opencode-rotation-test");
assertNoPrivateKeyLeak(successorSequenceTwoBytes, newOpenCodeKeyPem);
assert.equal(
  verify(
    null,
    Buffer.from(canonicalPayload(successorSequenceTwo)),
    newOpenCodePublicKey,
    Buffer.from(successorSequenceTwo.signature.value, "base64"),
  ),
  true,
);
assert.equal(
  verify(
    null,
    Buffer.from(canonicalPayload(successorSequenceTwo)),
    oldOpenCodePublicKey,
    Buffer.from(successorSequenceTwo.signature.value, "base64"),
  ),
  false,
);
assert.deepEqual(successorCheckpoints.opencode, {
  sequence: 2,
  payloadHash: payloadHash(successorSequenceTwo),
});
assert.deepEqual(
  JSON.parse(await readFile(path.join(successorOutput, "checkpoints.json"), "utf8"))
    .opencode,
  successorCheckpoints.opencode,
);

for (const fixture of [
  {
    name: "checkpoint-drift",
    histories: {
      opencode: [{ bundle: oldSignedOpenCode, bytes: oldOpenCodeBytes, checkpointHash: "a".repeat(64) }],
    },
    error: /checkpoint.*payload hash/i,
  },
  {
    name: "malformed-signature",
    histories: {
      opencode: [{ bundle: opencode, bytes: `${JSON.stringify(opencode)}\n` }],
    },
    error: /signature/i,
  },
]) {
  const fixtureOutput = path.join(scratch, fixture.name);
  stageHistory(fixtureOutput, fixture.histories);
  assert.throws(
    () =>
      buildCompatibilityRegistrySite({
        root,
        output: fixtureOutput,
        opencodePrivateKeyPem: privateKeyPem,
        grokPrivateKeyPem: grokKey,
        openclawPrivateKeyPem,
      }),
    fixture.error,
  );
}

const sourceDriftOutput = path.join(scratch, "source-drift");
const driftedOpenCode = signBundle(
  { ...opencode, expiresAt: "2031-01-01T00:00:00.000Z" },
  oldOpenCodeKey.export({ type: "pkcs8", format: "pem" }).toString(),
);
stageHistory(sourceDriftOutput, {
  opencode: [{ bundle: driftedOpenCode, bytes: `${JSON.stringify(driftedOpenCode)}\n` }],
});
assert.throws(
  () =>
    buildCompatibilityRegistrySite({
      root,
      output: sourceDriftOutput,
      opencodePrivateKeyPem: privateKeyPem,
      grokPrivateKeyPem: grokKey,
      openclawPrivateKeyPem,
    }),
  /reviewed source/i,
);

const missingHistoryOutput = path.join(scratch, "missing-history");
writeJson(path.join(missingHistoryOutput, "checkpoints.json"), {
  opencode: { sequence: 1, payloadHash: payloadHash(oldSignedOpenCode) },
});
assert.throws(
  () =>
    buildCompatibilityRegistrySite({
      root,
      output: missingHistoryOutput,
      opencodePrivateKeyPem: privateKeyPem,
      grokPrivateKeyPem: grokKey,
      openclawPrivateKeyPem,
    }),
  /missing historical.*opencode.*1/i,
);

const unexpectedHistoryOutput = path.join(scratch, "unexpected-history");
writeJson(path.join(unexpectedHistoryOutput, "checkpoints.json"), {});
mkdirSync(path.join(unexpectedHistoryOutput, "opencode"), { recursive: true });
writeFileSync(path.join(unexpectedHistoryOutput, "opencode/1.json"), oldOpenCodeBytes);
assert.throws(
  () =>
    buildCompatibilityRegistrySite({
      root,
      output: unexpectedHistoryOutput,
      opencodePrivateKeyPem: privateKeyPem,
      grokPrivateKeyPem: grokKey,
      openclawPrivateKeyPem,
    }),
  /unexpected pre-existing/i,
);

const gappedRoot = createSourceFixture("gapped-source", {
  opencode: [opencode, { ...opencode, sequence: 3 }],
  grok: [grok],
  openclaw: [openclaw],
});
const gappedOutput = path.join(scratch, "gapped-output");
writeJson(path.join(gappedOutput, "checkpoints.json"), {});
assert.throws(
  () =>
    buildCompatibilityRegistrySite({
      root: gappedRoot,
      output: gappedOutput,
      opencodePrivateKeyPem: privateKeyPem,
      grokPrivateKeyPem: grokKey,
      openclawPrivateKeyPem,
    }),
  /expected sequence 2/i,
);

let hydrationModule;
try {
  hydrationModule = await import("./hydrate-compatibility-registry-site.mjs");
} catch (error) {
  assert.fail(`compatibility registry hydration module must exist: ${error.code ?? error.message}`);
}
const {
  CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL,
  hydrateCompatibilityRegistrySite,
} = hydrationModule;
assert.equal(typeof hydrateCompatibilityRegistrySite, "function");
assert.equal(CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL, "https://opencoven.github.io/coven-runtimes");

function fakeResponse(url, status, body, { redirected = false } = {}) {
  return {
    status,
    redirected,
    url,
    async arrayBuffer() {
      return Uint8Array.from(Buffer.from(body)).buffer;
    },
  };
}

function fakeRegistryFetch(routes, calls = []) {
  return async (url, options) => {
    calls.push({ url, options });
    const route = routes.get(url);
    if (route instanceof Error) throw route;
    if (!route) return fakeResponse(url, 404, "not found");
    return fakeResponse(url, route.status, route.body, route);
  };
}

function checkpointRequestUrl(cacheBust) {
  const url = new URL(`${CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL}/checkpoints.json`);
  url.searchParams.set("hydration", cacheBust);
  return url.href;
}

const firstDeploymentOutput = path.join(scratch, "hydrated-first-deployment");
const firstDeploymentCalls = [];
const firstDeploymentCacheBust = "deterministic-first-deployment-run";
const firstDeploymentCheckpoints = await hydrateCompatibilityRegistrySite({
  output: firstDeploymentOutput,
  fetchImpl: fakeRegistryFetch(
    new Map([
      [
        checkpointRequestUrl(firstDeploymentCacheBust),
        { status: 404, body: "not found" },
      ],
    ]),
    firstDeploymentCalls,
  ),
  checkpointCacheBust: () => firstDeploymentCacheBust,
});
assert.deepEqual(firstDeploymentCheckpoints, {});
assert.deepEqual(JSON.parse(await readFile(path.join(firstDeploymentOutput, "checkpoints.json"), "utf8")), {});
assert.equal(firstDeploymentCalls.length, 1);
assert.equal(firstDeploymentCalls[0].url, checkpointRequestUrl(firstDeploymentCacheBust));
assert.equal(firstDeploymentCalls[0].options.redirect, "manual");
assert.equal(firstDeploymentCalls[0].options.cache, "no-store");
assert.equal(firstDeploymentCalls[0].options.headers["cache-control"], "no-cache, no-store, max-age=0");
assert.equal(firstDeploymentCalls[0].options.headers.pragma, "no-cache");

const secondHydrationCalls = [];
const secondHydrationCacheBust = "deterministic-second-hydration-run";
await hydrateCompatibilityRegistrySite({
  output: path.join(scratch, "hydrated-second-run"),
  fetchImpl: fakeRegistryFetch(
    new Map([
      [
        checkpointRequestUrl(secondHydrationCacheBust),
        { status: 404, body: "not found" },
      ],
    ]),
    secondHydrationCalls,
  ),
  checkpointCacheBust: () => secondHydrationCacheBust,
});
assert.notEqual(secondHydrationCalls[0].url, firstDeploymentCalls[0].url);

const productionCheckpoint = {
  opencode: { sequence: 1, payloadHash: payloadHash(oldSignedOpenCode) },
  grok: { sequence: 1, payloadHash: payloadHash(oldSignedGrok) },
};
const hydratedProductionOutput = path.join(scratch, "hydrated-production");
const productionHydrationCalls = [];
const productionCacheBust = "deterministic-production-run";
await hydrateCompatibilityRegistrySite({
  output: hydratedProductionOutput,
  fetchImpl: fakeRegistryFetch(
    new Map([
      [
        checkpointRequestUrl(productionCacheBust),
        { status: 200, body: `${JSON.stringify(productionCheckpoint)}\n` },
      ],
      [
        `${CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL}/opencode/1.json`,
        { status: 200, body: oldOpenCodeBytes },
      ],
      [
        `${CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL}/grok/1.json`,
        { status: 200, body: oldGrokBytes },
      ],
    ]),
    productionHydrationCalls,
  ),
  checkpointCacheBust: () => productionCacheBust,
});
assert.deepEqual(
  productionHydrationCalls.map(({ url }) => url),
  [
    checkpointRequestUrl(productionCacheBust),
    `${CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL}/opencode/1.json`,
    `${CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL}/grok/1.json`,
  ],
);
assert.equal(productionHydrationCalls[0].options.cache, "no-store");
for (const { options } of productionHydrationCalls.slice(1)) {
  assert.equal(Object.hasOwn(options, "cache"), false);
  assert.deepEqual(options.headers, { accept: "application/json" });
}
assert.equal(await readFile(path.join(hydratedProductionOutput, "opencode/1.json"), "utf8"), oldOpenCodeBytes);
assert.equal(await readFile(path.join(hydratedProductionOutput, "grok/1.json"), "utf8"), oldGrokBytes);
buildCompatibilityRegistrySite({
  root,
  output: hydratedProductionOutput,
  opencodePrivateKeyPem: generateKeyPairSync("ed25519").privateKey.export({ type: "pkcs8", format: "pem" }).toString(),
  grokPrivateKeyPem: generateKeyPairSync("ed25519").privateKey.export({ type: "pkcs8", format: "pem" }).toString(),
  openclawPrivateKeyPem,
});
assert.equal(await readFile(path.join(hydratedProductionOutput, "opencode/1.json"), "utf8"), oldOpenCodeBytes);
assert.equal(await readFile(path.join(hydratedProductionOutput, "grok/1.json"), "utf8"), oldGrokBytes);
assert.equal(
  await readFile(path.join(hydratedProductionOutput, "openclaw/current.json"), "utf8"),
  await readFile(path.join(hydratedProductionOutput, "openclaw/1.json"), "utf8"),
);

for (const fixture of [
  {
    name: "checkpoint-network-error",
    cacheBust: "checkpoint-network-error-run",
    routes: new Map([
      [
        checkpointRequestUrl("checkpoint-network-error-run"),
        new Error("simulated TLS failure"),
      ],
    ]),
    error: /failed to fetch canonical checkpoints/i,
  },
  {
    name: "checkpoint-redirect",
    cacheBust: "checkpoint-redirect-run",
    routes: new Map([
      [
        checkpointRequestUrl("checkpoint-redirect-run"),
        { status: 302, body: "", redirected: false },
      ],
    ]),
    error: /redirect|status 302/i,
  },
  {
    name: "checkpoint-followed-redirect",
    cacheBust: "checkpoint-followed-redirect-run",
    routes: new Map([
      [
        checkpointRequestUrl("checkpoint-followed-redirect-run"),
        { status: 200, body: "{}", redirected: true },
      ],
    ]),
    error: /redirect/i,
  },
  {
    name: "checkpoint-malformed-json",
    cacheBust: "checkpoint-malformed-json-run",
    routes: new Map([
      [
        checkpointRequestUrl("checkpoint-malformed-json-run"),
        { status: 200, body: "not-json" },
      ],
    ]),
    error: /checkpoints.*malformed/i,
  },
  {
    name: "checkpoint-empty-success",
    cacheBust: "checkpoint-empty-success-run",
    routes: new Map([
      [
        checkpointRequestUrl("checkpoint-empty-success-run"),
        { status: 200, body: "{}" },
      ],
    ]),
    error: /empty.*actual 404|actual 404.*empty/i,
  },
  {
    name: "checkpoint-unknown-runtime",
    cacheBust: "checkpoint-unknown-runtime-run",
    routes: new Map([
      [
        checkpointRequestUrl("checkpoint-unknown-runtime-run"),
        {
          status: 200,
          body: JSON.stringify({
            unknown: { sequence: 1, payloadHash: "a".repeat(64) },
          }),
        },
      ],
    ]),
    error: /unknown runtime/i,
  },
  {
    name: "checkpoint-invalid-sequence",
    cacheBust: "checkpoint-invalid-sequence-run",
    routes: new Map([
      [
        checkpointRequestUrl("checkpoint-invalid-sequence-run"),
        {
          status: 200,
          body: JSON.stringify({
            opencode: { sequence: 0, payloadHash: "a".repeat(64) },
          }),
        },
      ],
    ]),
    error: /checkpoint.*malformed/i,
  },
  {
    name: "checkpoint-invalid-hash",
    cacheBust: "checkpoint-invalid-hash-run",
    routes: new Map([
      [
        checkpointRequestUrl("checkpoint-invalid-hash-run"),
        {
          status: 200,
          body: JSON.stringify({
            opencode: { sequence: 1, payloadHash: "A".repeat(64) },
          }),
        },
      ],
    ]),
    error: /checkpoint.*malformed/i,
  },
  {
    name: "missing-historical-gap",
    cacheBust: "missing-historical-gap-run",
    routes: new Map([
      [
        checkpointRequestUrl("missing-historical-gap-run"),
        {
          status: 200,
          body: JSON.stringify({
            opencode: { sequence: 2, payloadHash: "a".repeat(64) },
          }),
        },
      ],
      [
        `${CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL}/opencode/1.json`,
        { status: 200, body: oldOpenCodeBytes },
      ],
    ]),
    error: /missing historical.*opencode.*2|status 404/i,
  },
]) {
  await assert.rejects(
    () =>
      hydrateCompatibilityRegistrySite({
        output: path.join(scratch, fixture.name),
        fetchImpl: fakeRegistryFetch(fixture.routes),
        checkpointCacheBust: () => fixture.cacheBust,
      }),
    fixture.error,
  );
}

const workflow = readFileSync(new URL("../.github/workflows/publish-compatibility-registry.yml", import.meta.url), "utf8");
assert.match(workflow, /workflow_dispatch/);
assert.doesNotMatch(workflow, /inputs:/, "publisher never checks out caller-selected workflow code with signing keys");
assert.match(workflow, /compatibility-registry-publisher/);
assert.match(workflow, /ref: main/);
assert.match(workflow, /COMPATIBILITY_OPENCODE_PRIVATE_KEY_PEM/);
assert.match(workflow, /COMPATIBILITY_GROK_PRIVATE_KEY_PEM/);
assert.match(workflow, /COMPATIBILITY_OPENCLAW_PRIVATE_KEY_PEM/);
assert.match(workflow, /actions\/upload-pages-artifact/);
assert.match(workflow, /actions\/deploy-pages/);
const hydrationCommandIndex = workflow.indexOf("node scripts/hydrate-compatibility-registry-site.mjs");
const buildCommandIndex = workflow.indexOf("node scripts/build-compatibility-registry-site.mjs");
assert.ok(hydrationCommandIndex >= 0, "publisher workflow must hydrate immutable history");
assert.ok(hydrationCommandIndex < buildCommandIndex, "publisher workflow must hydrate before signing");
const hydrationSource = readFileSync(
  new URL("./hydrate-compatibility-registry-site.mjs", import.meta.url),
  "utf8",
);
assert.doesNotMatch(hydrationSource, /PRIVATE_KEY|process\.env/, "hydration must not receive signing material");

const ci = readFileSync(new URL("../.github/workflows/ci.yml", import.meta.url), "utf8");
assert.match(ci, /Compatibility registry/);
assert.match(ci, /node scripts\/compatibility-registry\.test\.mjs/);

const runbook = readFileSync(new URL("../docs/compatibility-registry-publisher.md", import.meta.url), "utf8");
assert.match(runbook, /opencoven\.github\.io\/coven-runtimes\/opencode\/current\.json/);
assert.match(runbook, /opencoven\.github\.io\/coven-runtimes\/openclaw\/current\.json/);
assert.match(runbook, /COMPATIBILITY_OPENCLAW_PRIVATE_KEY_PEM/);
assert.match(runbook, /OPENCODE_SCHEMA_REGISTRY_CHECKPOINT/);
assert.match(runbook, /GROK_SCHEMA_REGISTRY_CHECKPOINT/);
assert.match(runbook, /OPENCLAW_SCHEMA_REGISTRY_CHECKPOINT/);
assert.match(runbook, /hydrate/i);
assert.match(runbook, /byte-for-byte/i);
assert.match(runbook, /key rotation/i);
assert.match(runbook, /actual 404/i);
assert.match(runbook, /redirect/i);
assert.match(runbook, /fail closed/i);

console.log("compatibility registry source contracts: pass");

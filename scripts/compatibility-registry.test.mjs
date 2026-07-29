import assert from "node:assert/strict";
import { generateKeyPairSync, verify } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { canonicalPayload, payloadHash, readUnsignedBundle, signBundle } from "./compatibility-registry.mjs";
import { buildCompatibilityRegistrySite } from "./build-compatibility-registry-site.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));

function readSource(relativePath) {
  return JSON.parse(readFileSync(path.join(root, relativePath), "utf8"));
}

function assertUnsignedSource(bundle, runtime) {
  assert.equal(bundle.format, 1);
  assert.equal(bundle.runtime, runtime);
  assert.equal(bundle.sequence, 1);
  assert.equal(Object.hasOwn(bundle, "signature"), false);
  assert.equal(Object.hasOwn(bundle, "keyId"), false);
  assert.ok(Array.isArray(bundle.schemas) && bundle.schemas.length > 0);
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

const opencode = readSource("registry/compatibility/opencode/1.json");
const grok = readSource("registry/compatibility/grok/1.json");

assertUnsignedSource(opencode, "opencode");
assertUnsignedSource(grok, "grok-build");

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
assert.deepEqual(readUnsignedBundle(path.join(root, "registry/compatibility/grok/1.json")), grok);

const grokKey = generateKeyPairSync("ed25519").privateKey.export({ type: "pkcs8", format: "pem" }).toString();
const output = await mkdtemp(path.join(tmpdir(), "compatibility-registry-site-"));
const checkpoints = buildCompatibilityRegistrySite({
  root,
  output,
  opencodePrivateKeyPem: privateKeyPem,
  grokPrivateKeyPem: grokKey,
});
const opencodeCurrent = await readFile(path.join(output, "opencode/current.json"), "utf8");
const opencodeLatest = await readFile(latestSequencePath(path.join(output, "opencode")), "utf8");
const grokCurrent = await readFile(path.join(output, "grok/current.json"), "utf8");
const grokLatest = await readFile(latestSequencePath(path.join(output, "grok")), "utf8");
assert.equal(opencodeCurrent, opencodeLatest);
assert.equal(grokCurrent, grokLatest);
assert.equal(JSON.parse(opencodeCurrent).keyId, undefined, "genesis payload must match Cave's pinned sequence-one contract");
assert.equal(JSON.parse(grokCurrent).keyId, undefined, "genesis payload must match Cave's pinned sequence-one contract");
assert.deepEqual(JSON.parse(await readFile(path.join(output, "checkpoints.json"), "utf8")), checkpoints);
assertNoPrivateKeyLeak(opencodeCurrent, privateKeyPem);
assertNoPrivateKeyLeak(grokCurrent, grokKey);

const workflow = readFileSync(new URL("../.github/workflows/publish-compatibility-registry.yml", import.meta.url), "utf8");
assert.match(workflow, /workflow_dispatch/);
assert.doesNotMatch(workflow, /inputs:/, "publisher never checks out caller-selected workflow code with signing keys");
assert.match(workflow, /compatibility-registry-publisher/);
assert.match(workflow, /ref: main/);
assert.match(workflow, /COMPATIBILITY_OPENCODE_PRIVATE_KEY_PEM/);
assert.match(workflow, /COMPATIBILITY_GROK_PRIVATE_KEY_PEM/);
assert.match(workflow, /actions\/upload-pages-artifact/);
assert.match(workflow, /actions\/deploy-pages/);

const ci = readFileSync(new URL("../.github/workflows/ci.yml", import.meta.url), "utf8");
assert.match(ci, /Compatibility registry/);
assert.match(ci, /node scripts\/compatibility-registry\.test\.mjs/);

const runbook = readFileSync(new URL("../docs/compatibility-registry-publisher.md", import.meta.url), "utf8");
assert.match(runbook, /opencoven\.github\.io\/coven-runtimes\/opencode\/current\.json/);
assert.match(runbook, /OPENCODE_SCHEMA_REGISTRY_CHECKPOINT/);
assert.match(runbook, /GROK_SCHEMA_REGISTRY_CHECKPOINT/);

console.log("compatibility registry source contracts: pass");

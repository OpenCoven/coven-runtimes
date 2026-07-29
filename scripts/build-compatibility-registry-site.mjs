import { mkdirSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { payloadHash, readUnsignedBundle, signBundle } from "./compatibility-registry.mjs";

const RUNTIMES = [
  { name: "opencode", source: "opencode", key: "opencodePrivateKeyPem" },
  { name: "grok", source: "grok", key: "grokPrivateKeyPem" },
];

function sourceFiles(root, source) {
  const directory = path.join(root, "registry", "compatibility", source);
  return readdirSync(directory)
    .filter((file) => /^\d+\.json$/.test(file))
    .sort((left, right) => Number(path.basename(left, ".json")) - Number(path.basename(right, ".json")))
    .map((file) => path.join(directory, file));
}

function writeJson(file, value) {
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

export function buildCompatibilityRegistrySite({ root, output, ...keys }) {
  const checkpoints = {};
  for (const runtime of RUNTIMES) {
    const privateKeyPem = keys[runtime.key];
    if (!privateKeyPem) throw new Error(`missing ${runtime.name} compatibility registry signing material`);
    const sourceFilesWithSequence = sourceFiles(root, runtime.source);
    let expectedSequence = 1;
    const signed = sourceFilesWithSequence.map((file) => {
      const bundle = readUnsignedBundle(file);
      const sequenceFromFilename = Number(path.basename(file, ".json"));
      if (!Number.isSafeInteger(sequenceFromFilename)) {
        throw new TypeError(`compatibility registry source filename ${file} must be a safe integer sequence number`);
      }
      if (bundle.sequence !== sequenceFromFilename) {
        throw new TypeError(`compatibility registry sequence mismatch in ${file}; filename implies ${sequenceFromFilename} but payload has ${bundle.sequence}`);
      }
      if (bundle.sequence < expectedSequence) {
        throw new TypeError(`compatibility registry sequence must be strictly increasing for ${runtime.name}`);
      }
      expectedSequence = bundle.sequence + 1;
      return signBundle(bundle, privateKeyPem);
    });
    if (!signed.length) throw new Error(`no ${runtime.name} compatibility registry sources found`);
    for (const bundle of signed) writeJson(path.join(output, runtime.name, `${bundle.sequence}.json`), bundle);
    const latest = signed.at(-1);
    writeJson(path.join(output, runtime.name, "current.json"), latest);
    checkpoints[runtime.name] = { sequence: latest.sequence, payloadHash: payloadHash(latest) };
  }
  writeJson(path.join(output, "checkpoints.json"), checkpoints);
  return checkpoints;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL("..", import.meta.url));
  buildCompatibilityRegistrySite({
    root,
    output: path.join(root, "site"),
    opencodePrivateKeyPem: process.env.COMPATIBILITY_OPENCODE_PRIVATE_KEY_PEM,
    grokPrivateKeyPem: process.env.COMPATIBILITY_GROK_PRIVATE_KEY_PEM,
  });
}

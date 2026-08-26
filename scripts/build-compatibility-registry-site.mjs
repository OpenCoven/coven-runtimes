import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  canonicalPayload,
  parseSignedBundle,
  payloadHash,
  readUnsignedBundle,
  signBundle,
} from "./compatibility-registry.mjs";
import {
  COMPATIBILITY_REGISTRY_PUBLISHERS,
  validateCompatibilityRegistryCheckpoints,
} from "./compatibility-registry-publication.mjs";

const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });

function sourceFiles(root, source) {
  const directory = path.join(root, "registry", "compatibility", source);
  return readdirSync(directory)
    .filter((file) => /^\d+\.json$/.test(file))
    .sort((left, right) => Number(path.basename(left, ".json")) - Number(path.basename(right, ".json")))
    .map((file) => path.join(directory, file));
}

function jsonBytes(value) {
  return Buffer.from(`${JSON.stringify(value, null, 2)}\n`);
}

function writeBytes(file, bytes) {
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, bytes);
}

function readHydratedCheckpoints(output) {
  const checkpointFile = path.join(output, "checkpoints.json");
  if (!existsSync(checkpointFile)) {
    throw new Error("compatibility registry staging is missing hydrated checkpoints.json");
  }
  let checkpoints;
  try {
    checkpoints = JSON.parse(UTF8_DECODER.decode(readFileSync(checkpointFile)));
  } catch (error) {
    throw new TypeError(`compatibility registry hydrated checkpoints are malformed: ${error.message}`);
  }
  return validateCompatibilityRegistryCheckpoints(checkpoints);
}

function collectStagingEntries(directory, relative = "") {
  const entries = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const relativePath = relative ? `${relative}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      entries.push({ path: relativePath, type: "directory" });
      entries.push(...collectStagingEntries(path.join(directory, entry.name), relativePath));
    } else if (entry.isFile()) {
      entries.push({ path: relativePath, type: "file" });
    } else {
      entries.push({ path: relativePath, type: "unsupported" });
    }
  }
  return entries;
}

function validateStagingInventory(output, checkpoints) {
  const expectedFiles = new Set(["checkpoints.json"]);
  const expectedDirectories = new Set();
  for (const [runtime, checkpoint] of Object.entries(checkpoints)) {
    expectedDirectories.add(runtime);
    for (let sequence = 1; sequence <= checkpoint.sequence; sequence += 1) {
      expectedFiles.add(`${runtime}/${sequence}.json`);
    }
  }

  const entries = collectStagingEntries(output);
  for (const entry of entries) {
    const expected = entry.type === "file" ? expectedFiles.has(entry.path) : expectedDirectories.has(entry.path);
    if (!expected) {
      throw new Error(`compatibility registry staging contains unexpected pre-existing ${entry.type} ${entry.path}`);
    }
  }
  for (const expectedFile of expectedFiles) {
    if (expectedFile === "checkpoints.json") continue;
    if (!entries.some((entry) => entry.type === "file" && entry.path === expectedFile)) {
      const [runtime, filename] = expectedFile.split("/");
      throw new Error(
        `compatibility registry staging is missing historical ${runtime} sequence ${path.basename(filename, ".json")}`,
      );
    }
  }
}

function parseHydratedBundle(file) {
  const bytes = readFileSync(file);
  let serialized;
  try {
    serialized = UTF8_DECODER.decode(bytes);
  } catch (error) {
    throw new TypeError(`compatibility registry historical file ${file} is not valid UTF-8: ${error.message}`);
  }
  let bundle;
  try {
    bundle = parseSignedBundle(serialized);
  } catch (error) {
    throw new TypeError(`compatibility registry historical file ${file} is invalid: ${error.message}`);
  }
  return { bundle, bytes };
}

function prepareRuntime(root, output, publisher, checkpoint) {
  const files = sourceFiles(root, publisher.source);
  if (!files.length) throw new Error(`no ${publisher.name} compatibility registry sources found`);

  const sources = files.map((file, index) => {
    const sequenceFromFilename = Number(path.basename(file, ".json"));
    const expectedSequence = index + 1;
    if (
      !Number.isSafeInteger(sequenceFromFilename) ||
      path.basename(file) !== `${sequenceFromFilename}.json`
    ) {
      throw new TypeError(
        `compatibility registry source filename ${file} must be a canonical safe integer sequence number`,
      );
    }
    if (sequenceFromFilename !== expectedSequence) {
      throw new TypeError(
        `compatibility registry source gap for ${publisher.name}; expected sequence ${expectedSequence} but found ${sequenceFromFilename}`,
      );
    }

    const bundle = readUnsignedBundle(file);
    if (bundle.runtime !== publisher.payloadRuntime) {
      throw new TypeError(
        `compatibility registry source runtime mismatch for ${publisher.source}; expected ${publisher.payloadRuntime} but found ${bundle.runtime}`,
      );
    }
    if (bundle.sequence !== sequenceFromFilename) {
      throw new TypeError(
        `compatibility registry sequence mismatch in ${file}; filename implies ${sequenceFromFilename} but payload has ${bundle.sequence}`,
      );
    }
    return { bundle, file };
  });

  if (checkpoint && checkpoint.sequence > sources.length) {
    throw new Error(
      `compatibility registry reviewed sources for ${publisher.name} end at sequence ${sources.length}, below hydrated publication sequence ${checkpoint.sequence}`,
    );
  }

  const preserved = new Map();
  if (checkpoint) {
    for (let sequence = 1; sequence <= checkpoint.sequence; sequence += 1) {
      const historicalFile = path.join(output, publisher.name, `${sequence}.json`);
      const historical = parseHydratedBundle(historicalFile);
      if (
        historical.bundle.runtime !== publisher.payloadRuntime ||
        historical.bundle.sequence !== sequence
      ) {
        throw new TypeError(
          `compatibility registry historical runtime/sequence mismatch for ${publisher.name} sequence ${sequence}`,
        );
      }
      if (canonicalPayload(historical.bundle) !== canonicalPayload(sources[sequence - 1].bundle)) {
        throw new Error(
          `compatibility registry historical ${publisher.name} sequence ${sequence} does not match the reviewed source`,
        );
      }
      if (
        sequence === checkpoint.sequence &&
        payloadHash(historical.bundle) !== checkpoint.payloadHash
      ) {
        throw new Error(
          `compatibility registry checkpoint payload hash does not match hydrated ${publisher.name} sequence ${sequence}`,
        );
      }
      preserved.set(sequence, historical);
    }
  }

  return { publisher, sources, preserved, checkpoint };
}

export function buildCompatibilityRegistrySite({ root, output, ...keys }) {
  const hydratedCheckpoints = readHydratedCheckpoints(output);
  validateStagingInventory(output, hydratedCheckpoints);

  const plans = COMPATIBILITY_REGISTRY_PUBLISHERS.map((publisher) =>
    prepareRuntime(root, output, publisher, hydratedCheckpoints[publisher.name]),
  );
  for (const plan of plans) {
    if (plan.sources.length > (plan.checkpoint?.sequence ?? 0) && !keys[plan.publisher.key]) {
      throw new Error(`missing ${plan.publisher.name} compatibility registry signing material`);
    }
  }

  const checkpoints = {};
  for (const { publisher, sources, preserved, checkpoint } of plans) {
    let latestBundle;
    let latestBytes;
    for (const { bundle } of sources) {
      const historical = preserved.get(bundle.sequence);
      if (historical) {
        latestBundle = historical.bundle;
        latestBytes = historical.bytes;
        continue;
      }
      if (checkpoint && bundle.sequence <= checkpoint.sequence) {
        throw new Error(
          `compatibility registry staging is missing historical ${publisher.name} sequence ${bundle.sequence}`,
        );
      }
      latestBundle = signBundle(bundle, keys[publisher.key]);
      latestBytes = jsonBytes(latestBundle);
      writeBytes(path.join(output, publisher.name, `${bundle.sequence}.json`), latestBytes);
    }

    writeBytes(path.join(output, publisher.name, "current.json"), latestBytes);
    checkpoints[publisher.name] = {
      sequence: latestBundle.sequence,
      payloadHash: payloadHash(latestBundle),
    };
  }
  writeBytes(path.join(output, "checkpoints.json"), jsonBytes(checkpoints));
  return checkpoints;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL("..", import.meta.url));
  buildCompatibilityRegistrySite({
    root,
    output: path.join(root, "site"),
    opencodePrivateKeyPem: process.env.COMPATIBILITY_OPENCODE_PRIVATE_KEY_PEM,
    grokPrivateKeyPem: process.env.COMPATIBILITY_GROK_PRIVATE_KEY_PEM,
    openclawPrivateKeyPem: process.env.COMPATIBILITY_OPENCLAW_PRIVATE_KEY_PEM,
  });
}

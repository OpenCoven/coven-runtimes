import { readFileSync } from "node:fs";
import path from "node:path";

const PUBLISHERS = [
  {
    name: "opencode",
    source: "opencode",
    payloadRuntime: "opencode",
    key: "opencodePrivateKeyPem",
  },
  {
    name: "grok",
    source: "grok",
    payloadRuntime: "grok-build",
    key: "grokPrivateKeyPem",
  },
  {
    name: "openclaw",
    source: "openclaw",
    payloadRuntime: "openclaw",
    key: "openclawPrivateKeyPem",
  },
];

export const COMPATIBILITY_REGISTRY_PUBLISHERS = Object.freeze(
  PUBLISHERS.map((publisher) => Object.freeze(publisher)),
);

export const COMPATIBILITY_REGISTRY_PUBLICATION_STATE_RELATIVE_PATH =
  "registry/compatibility/publication-state.json";

const PUBLISHER_NAMES = new Set(PUBLISHERS.map(({ name }) => name));
const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });

function isRecord(value) {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

export function validateCompatibilityRegistryPublicationState(value) {
  if (!isRecord(value)) {
    throw new TypeError("compatibility registry publication state must be an object");
  }
  const keys = Object.keys(value).sort();
  const expectedKeys = [
    "authorizedGenesisRuntimes",
    "publishedArtifactHashes",
    "publishedCheckpoints",
    "publishedSequenceFloors",
  ];
  if (
    keys.length !== expectedKeys.length ||
    keys.some((key, index) => key !== expectedKeys[index])
  ) {
    throw new TypeError(
      "compatibility registry publication state must contain exactly publishedSequenceFloors, publishedCheckpoints, publishedArtifactHashes, and authorizedGenesisRuntimes",
    );
  }

  const {
    publishedSequenceFloors,
    publishedCheckpoints,
    publishedArtifactHashes,
    authorizedGenesisRuntimes,
  } = value;
  if (!isRecord(publishedSequenceFloors)) {
    throw new TypeError(
      "compatibility registry published sequence floors must be an object",
    );
  }
  if (!isRecord(publishedCheckpoints)) {
    throw new TypeError(
      "compatibility registry published checkpoints must be an object",
    );
  }
  if (!isRecord(publishedArtifactHashes)) {
    throw new TypeError(
      "compatibility registry published artifact hashes must be an object",
    );
  }
  if (!Array.isArray(authorizedGenesisRuntimes)) {
    throw new TypeError(
      "compatibility registry authorized genesis runtimes must be an array",
    );
  }

  const classifiedRuntimes = new Set();
  for (const [runtime, floor] of Object.entries(publishedSequenceFloors)) {
    if (!PUBLISHER_NAMES.has(runtime)) {
      throw new TypeError(
        `compatibility registry publication state contains unknown runtime ${runtime}`,
      );
    }
    if (!Number.isSafeInteger(floor) || floor < 1) {
      throw new TypeError(
        `compatibility registry publication floor for ${runtime} must be a positive safe integer`,
      );
    }
    const checkpoint = publishedCheckpoints[runtime];
    if (
      !isRecord(checkpoint) ||
      Object.keys(checkpoint).length !== 2 ||
      checkpoint.sequence !== floor ||
      typeof checkpoint.payloadHash !== "string" ||
      !/^[a-f0-9]{64}$/.test(checkpoint.payloadHash)
    ) {
      throw new TypeError(
        `compatibility registry published checkpoint for ${runtime} must match floor ${floor}`,
      );
    }
    const hashes = publishedArtifactHashes[runtime];
    if (!isRecord(hashes)) {
      throw new TypeError(
        `compatibility registry published artifact hashes for ${runtime} must be an object`,
      );
    }
    const actualSequences = Object.keys(hashes).sort((left, right) => Number(left) - Number(right));
    if (
      actualSequences.length !== floor ||
      actualSequences.some((sequence, index) => sequence !== String(index + 1)) ||
      actualSequences.some((sequence) => !/^[a-f0-9]{64}$/.test(hashes[sequence]))
    ) {
      throw new TypeError(
        `compatibility registry published artifact hashes for ${runtime} must cover every sequence through ${floor}`,
      );
    }
    classifiedRuntimes.add(runtime);
  }
  for (const runtime of Object.keys(publishedCheckpoints)) {
    if (!Object.hasOwn(publishedSequenceFloors, runtime)) {
      throw new TypeError(
        `compatibility registry published checkpoints contain unclassified runtime ${runtime}`,
      );
    }
  }
  for (const runtime of Object.keys(publishedArtifactHashes)) {
    if (!Object.hasOwn(publishedSequenceFloors, runtime)) {
      throw new TypeError(
        `compatibility registry published artifact hashes contain unclassified runtime ${runtime}`,
      );
    }
  }

  const genesisRuntimes = new Set();
  for (const runtime of authorizedGenesisRuntimes) {
    if (typeof runtime !== "string" || !PUBLISHER_NAMES.has(runtime)) {
      throw new TypeError(
        `compatibility registry publication state contains unknown runtime ${String(runtime)}`,
      );
    }
    if (genesisRuntimes.has(runtime)) {
      throw new TypeError(
        `compatibility registry publication state contains duplicate authorized genesis runtime ${runtime}`,
      );
    }
    if (classifiedRuntimes.has(runtime)) {
      throw new TypeError(
        `compatibility registry publication state has floor/genesis overlap for ${runtime}`,
      );
    }
    genesisRuntimes.add(runtime);
    classifiedRuntimes.add(runtime);
  }

  for (const runtime of PUBLISHER_NAMES) {
    if (!classifiedRuntimes.has(runtime)) {
      throw new TypeError(
        `compatibility registry publication state does not classify runtime ${runtime}`,
      );
    }
  }
  return value;
}

export function readCompatibilityRegistryPublicationState(root) {
  const file = path.join(
    root,
    COMPATIBILITY_REGISTRY_PUBLICATION_STATE_RELATIVE_PATH,
  );
  let bytes;
  try {
    bytes = readFileSync(file);
  } catch (error) {
    throw new Error(
      `compatibility registry publication state could not be read: ${error.message}`,
    );
  }

  let state;
  try {
    state = JSON.parse(UTF8_DECODER.decode(bytes));
  } catch (error) {
    throw new TypeError(
      `compatibility registry publication state is malformed: ${error.message}`,
    );
  }
  return validateCompatibilityRegistryPublicationState(state);
}

export function validateCompatibilityRegistryCheckpoints(
  value,
  publicationState,
) {
  const state = validateCompatibilityRegistryPublicationState(publicationState);
  if (!isRecord(value)) {
    throw new TypeError("compatibility registry checkpoints must be an object");
  }
  for (const [runtime, checkpoint] of Object.entries(value)) {
    if (!PUBLISHER_NAMES.has(runtime)) {
      throw new TypeError(`compatibility registry checkpoints contain unknown runtime ${runtime}`);
    }
    if (
      !isRecord(checkpoint) ||
      Object.keys(checkpoint).length !== 2 ||
      !Number.isSafeInteger(checkpoint.sequence) ||
      checkpoint.sequence < 1 ||
      typeof checkpoint.payloadHash !== "string" ||
      !/^[a-f0-9]{64}$/.test(checkpoint.payloadHash)
    ) {
      throw new TypeError(`compatibility registry checkpoint for ${runtime} is malformed`);
    }
  }

  for (const [runtime, floor] of Object.entries(
    state.publishedSequenceFloors,
  )) {
    const checkpoint = value[runtime];
    const expected = state.publishedCheckpoints[runtime];
    if (
      !checkpoint ||
      checkpoint.sequence !== floor ||
      checkpoint.payloadHash !== expected.payloadHash
    ) {
      throw new TypeError(
        `compatibility registry checkpoint for ${runtime} does not match tracked published sequence ${floor} and payload hash`,
      );
    }
  }
  for (const runtime of state.authorizedGenesisRuntimes) {
    if (Object.hasOwn(value, runtime)) {
      throw new TypeError(
        `compatibility registry runtime ${runtime} is authorized genesis but already published in checkpoints`,
      );
    }
  }
  return value;
}

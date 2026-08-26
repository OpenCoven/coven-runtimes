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

const PUBLISHER_NAMES = new Set(PUBLISHERS.map(({ name }) => name));

function isRecord(value) {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

export function validateCompatibilityRegistryCheckpoints(value) {
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
  return value;
}

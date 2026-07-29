import { createHash, createPrivateKey, sign } from "node:crypto";
import { readFileSync } from "node:fs";

function isRecord(value) {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (isRecord(value)) return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonicalize(value[key])]));
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number" && Number.isFinite(value)) return value;
  throw new TypeError("compatibility registry payload contains a non-JSON value");
}

function assertUnsignedBundle(bundle) {
  if (!isRecord(bundle) || bundle.format !== 1 || !["opencode", "grok-build"].includes(bundle.runtime)) {
    throw new TypeError("compatibility registry source must be a format-1 OpenCode or Grok bundle");
  }
  if (!Object.keys(bundle).every((key) => ["format", "runtime", "sequence", "issuedAt", "expiresAt", "keyId", "retiredSchemaIds", "schemas"].includes(key))) {
    throw new TypeError("compatibility registry source contains an unsupported field");
  }
  for (const [key, value] of Object.entries(bundle)) {
    if (value === undefined) throw new TypeError(`compatibility registry source field ${key} must not be undefined`);
  }
  if (!Number.isSafeInteger(bundle.sequence) || bundle.sequence < 1 || !Array.isArray(bundle.schemas) || bundle.schemas.length === 0) {
    throw new TypeError("compatibility registry source needs a positive sequence and non-empty schemas");
  }
  if (Object.hasOwn(bundle, "signature")) throw new TypeError("compatibility registry source must not contain a signature");
  if (bundle.keyId !== undefined && !/^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(bundle.keyId)) {
    throw new TypeError("compatibility registry source key id is invalid");
  }
}

export function canonicalPayload(bundle) {
  if (!isRecord(bundle)) throw new TypeError("compatibility registry bundle must be an object");
  const { signature: _signature, ...unsigned } = bundle;
  return JSON.stringify(canonicalize(unsigned));
}

export function payloadHash(bundle) {
  return createHash("sha256").update(canonicalPayload(bundle)).digest("hex");
}

export function readUnsignedBundle(file) {
  const bundle = JSON.parse(readFileSync(file, "utf8"));
  assertUnsignedBundle(bundle);
  return bundle;
}

export function signBundle(bundle, privateKeyPem) {
  assertUnsignedBundle(bundle);
  const key = createPrivateKey(privateKeyPem);
  if (key.asymmetricKeyType !== "ed25519") throw new TypeError("compatibility registry signing key must be Ed25519");
  const signed = { ...bundle };
  return {
    ...signed,
    signature: { algorithm: "ed25519", value: sign(null, Buffer.from(canonicalPayload(signed)), key).toString("base64") },
  };
}

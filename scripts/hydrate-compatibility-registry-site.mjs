import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { validateCompatibilityRegistryCheckpoints } from "./compatibility-registry-publication.mjs";

export const CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL =
  "https://opencoven.github.io/coven-runtimes";

const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });

function assertEmptyStagingDirectory(output) {
  if (existsSync(output) && readdirSync(output).length > 0) {
    throw new Error("compatibility registry hydration requires an empty output staging directory");
  }
}

async function fetchCanonicalBytes(fetchImpl, url, label, { allowNotFound = false } = {}) {
  let response;
  try {
    response = await fetchImpl(url, {
      method: "GET",
      redirect: "manual",
      headers: { accept: "application/json" },
    });
  } catch (error) {
    throw new Error(`failed to fetch canonical ${label}: ${error.message}`);
  }

  if (
    response.redirected ||
    (typeof response.url === "string" && response.url !== url) ||
    (response.status >= 300 && response.status < 400)
  ) {
    throw new Error(`canonical ${label} returned a redirect, which is forbidden`);
  }
  if (allowNotFound && response.status === 404) return null;
  if (response.status !== 200) {
    throw new Error(`canonical ${label} returned status ${response.status}`);
  }

  try {
    return Buffer.from(await response.arrayBuffer());
  } catch (error) {
    throw new Error(`failed to read canonical ${label}: ${error.message}`);
  }
}

function parseCheckpoints(bytes) {
  let checkpoints;
  try {
    checkpoints = JSON.parse(UTF8_DECODER.decode(bytes));
  } catch (error) {
    throw new TypeError(`canonical compatibility registry checkpoints are malformed: ${error.message}`);
  }
  const validated = validateCompatibilityRegistryCheckpoints(checkpoints);
  if (Object.keys(validated).length === 0) {
    throw new TypeError(
      "canonical compatibility registry checkpoints are empty; first deployment requires an actual 404",
    );
  }
  return validated;
}

function writeBytes(file, bytes) {
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, bytes);
}

export async function hydrateCompatibilityRegistrySite({ output, fetchImpl = globalThis.fetch }) {
  if (typeof fetchImpl !== "function") {
    throw new TypeError("compatibility registry hydration requires a fetch implementation");
  }
  assertEmptyStagingDirectory(output);

  const checkpointUrl = `${CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL}/checkpoints.json`;
  const checkpointBytes = await fetchCanonicalBytes(fetchImpl, checkpointUrl, "checkpoints", {
    allowNotFound: true,
  });
  if (checkpointBytes === null) {
    writeBytes(path.join(output, "checkpoints.json"), Buffer.from("{}\n"));
    return {};
  }

  const checkpoints = parseCheckpoints(checkpointBytes);
  const historicalFiles = [];
  for (const [runtime, checkpoint] of Object.entries(checkpoints)) {
    for (let sequence = 1; sequence <= checkpoint.sequence; sequence += 1) {
      const url = `${CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL}/${runtime}/${sequence}.json`;
      const bytes = await fetchCanonicalBytes(
        fetchImpl,
        url,
        `${runtime} historical sequence ${sequence}`,
      );
      historicalFiles.push({ runtime, sequence, bytes });
    }
  }

  writeBytes(path.join(output, "checkpoints.json"), checkpointBytes);
  for (const { runtime, sequence, bytes } of historicalFiles) {
    writeBytes(path.join(output, runtime, `${sequence}.json`), bytes);
  }
  return checkpoints;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL("..", import.meta.url));
  await hydrateCompatibilityRegistrySite({ output: path.join(root, "site") });
}

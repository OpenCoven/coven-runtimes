import { randomUUID } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  readCompatibilityRegistryPublicationState,
  validateCompatibilityRegistryCheckpoints,
} from "./compatibility-registry-publication.mjs";

export const CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL =
  "https://opencoven.github.io/coven-runtimes";

const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });

function assertEmptyStagingDirectory(output) {
  if (existsSync(output) && readdirSync(output).length > 0) {
    throw new Error("compatibility registry hydration requires an empty output staging directory");
  }
}

async function fetchCanonicalBytes(
  fetchImpl,
  url,
  label,
  { allowNotFound = false, noCache = false } = {},
) {
  let response;
  try {
    const headers = { accept: "application/json" };
    const options = {
      method: "GET",
      redirect: "manual",
      headers,
    };
    if (noCache) {
      options.cache = "no-store";
      headers["cache-control"] = "no-cache, no-store, max-age=0";
      headers.pragma = "no-cache";
    }
    response = await fetchImpl(url, {
      ...options,
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

function parseCheckpoints(bytes, publicationState) {
  let checkpoints;
  try {
    checkpoints = JSON.parse(UTF8_DECODER.decode(bytes));
  } catch (error) {
    throw new TypeError(`canonical compatibility registry checkpoints are malformed: ${error.message}`);
  }
  if (
    checkpoints &&
    typeof checkpoints === "object" &&
    !Array.isArray(checkpoints) &&
    Object.keys(checkpoints).length === 0
  ) {
    throw new TypeError(
      "canonical compatibility registry checkpoints are empty; first deployment requires an actual 404",
    );
  }
  return validateCompatibilityRegistryCheckpoints(checkpoints, publicationState);
}

function writeBytes(file, bytes) {
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, bytes);
}

export function parseHydrationCliArgs(args) {
  if (!Array.isArray(args) || args.some((argument) => typeof argument !== "string")) {
    throw new TypeError("compatibility registry hydration CLI arguments must be strings");
  }
  if (args.length === 0) return { allowBootstrap404: false };
  if (args.length === 1 && args[0] === "--allow-bootstrap-404") {
    return { allowBootstrap404: true };
  }
  throw new TypeError(
    "compatibility registry hydration accepts only the one-time --allow-bootstrap-404 flag",
  );
}

export async function hydrateCompatibilityRegistrySite({
  output,
  fetchImpl = globalThis.fetch,
  checkpointCacheBust = randomUUID,
  allowBootstrap404 = false,
}) {
  if (typeof fetchImpl !== "function") {
    throw new TypeError("compatibility registry hydration requires a fetch implementation");
  }
  if (typeof checkpointCacheBust !== "function") {
    throw new TypeError("compatibility registry hydration requires a checkpoint cache-bust factory");
  }
  if (typeof allowBootstrap404 !== "boolean") {
    throw new TypeError("compatibility registry bootstrap authorization must be a boolean");
  }
  const root = fileURLToPath(new URL("..", import.meta.url));
  const publicationState = readCompatibilityRegistryPublicationState(root);
  assertEmptyStagingDirectory(output);

  const cacheBust = checkpointCacheBust();
  if (typeof cacheBust !== "string" || cacheBust.length === 0) {
    throw new TypeError("compatibility registry checkpoint cache bust must be a non-empty string");
  }
  const checkpointUrl = new URL(
    `${CANONICAL_COMPATIBILITY_REGISTRY_BASE_URL}/checkpoints.json`,
  );
  checkpointUrl.searchParams.set("hydration", cacheBust);
  const checkpointBytes = await fetchCanonicalBytes(fetchImpl, checkpointUrl.href, "checkpoints", {
    allowNotFound: allowBootstrap404,
    noCache: true,
  });
  if (checkpointBytes === null) {
    writeBytes(path.join(output, "checkpoints.json"), Buffer.from("{}\n"));
    return {};
  }

  const checkpoints = parseCheckpoints(checkpointBytes, publicationState);
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
  const { allowBootstrap404 } = parseHydrationCliArgs(process.argv.slice(2));
  await hydrateCompatibilityRegistrySite({
    output: path.join(root, "site"),
    allowBootstrap404,
  });
}

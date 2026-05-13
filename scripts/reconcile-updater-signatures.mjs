#!/usr/bin/env node
// Reconcile Tauri updater signatures in latest.json against GitHub release .sig assets.
//
// Tauri expects latest.json platform.signature to be a base64-encoded minisign
// signature text. Depending on tauri-action/platform behavior, uploaded .sig
// assets can be either the raw minisign text or that same text already base64
// encoded, so normalize before comparing.

import fs from "node:fs";
import https from "node:https";
import crypto from "node:crypto";

const MAC_PLATFORMS = ["darwin-aarch64", "darwin-x86_64", "darwin-aarch64-app", "darwin-x86_64-app"];
const WINDOWS_PLATFORMS = ["windows-x86_64", "windows-x86_64-nsis"];
const REQUIRED_PLATFORMS = [...MAC_PLATFORMS, ...WINDOWS_PLATFORMS];
const TAURI_CONFIG_PATH = "src-tauri/tauri.conf.json";
const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function parseArgs() {
  const args = process.argv.slice(2);
  const parsed = {
    releasePath: "release.json",
    latestPath: "latest.json",
    markerPath: "needs_reupload",
  };
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === "--release") {
      parsed.releasePath = args[++i];
    } else if (arg === "--latest") {
      parsed.latestPath = args[++i];
    } else if (arg === "--marker") {
      parsed.markerPath = args[++i];
    } else {
      throw new Error(`Unexpected argument: ${arg}`);
    }
  }
  return parsed;
}

function looksLikeMinisignText(text) {
  return text.trimStart().startsWith("untrusted comment:");
}

function normalizeSignatureAsset(assetText) {
  const trimmed = assetText.trim();
  if (looksLikeMinisignText(trimmed)) {
    return Buffer.from(trimmed + "\n", "utf8").toString("base64");
  }

  try {
    const decoded = Buffer.from(trimmed, "base64").toString("utf8");
    if (looksLikeMinisignText(decoded)) {
      return trimmed;
    }
  } catch {
    // Fall through to the explicit error below.
  }

  throw new Error("Signature asset is neither raw minisign text nor base64-encoded minisign text");
}

function decodeLatestSignature(signature) {
  try {
    const decoded = Buffer.from(signature.trim(), "base64").toString("utf8");
    if (!looksLikeMinisignText(decoded)) {
      throw new Error("decoded text is not a minisign signature");
    }
    return decoded.trim();
  } catch (error) {
    throw new Error(`Invalid latest.json signature format: ${error.message}`);
  }
}

function displayRawSigLine(signature) {
  const lines = decodeLatestSignature(signature).split("\n").map((line) => line.trim()).filter(Boolean);
  return lines[1] || "(missing signature line)";
}

async function downloadAsset(asset, token) {
  return new Promise((resolve, reject) => {
    function get(url, withAuth) {
      const request = https.get(
        url,
        {
          headers: {
            Accept: "application/octet-stream",
            "User-Agent": "selah-ci",
            ...(withAuth ? { Authorization: `Bearer ${token}` } : {}),
          },
        },
        (response) => {
          if ([301, 302, 303, 307, 308].includes(response.statusCode || 0)) {
            response.resume();
            get(response.headers.location, false);
            return;
          }
          if ((response.statusCode || 0) >= 400) {
            reject(new Error(`Failed to download ${asset.name}: HTTP ${response.statusCode}`));
            return;
          }
          const chunks = [];
          response.on("data", (chunk) => {
            chunks.push(Buffer.from(chunk));
          });
          response.on("end", () => resolve(Buffer.concat(chunks)));
          response.on("error", reject);
        },
      );
      request.on("error", reject);
    }
    get(asset.url, true);
  });
}

async function downloadText(asset, token) {
  return (await downloadAsset(asset, token)).toString("utf8");
}

function filenameFromUrl(url) {
  try {
    return decodeURIComponent(new URL(url).pathname.split("/").pop() || "");
  } catch (error) {
    throw new Error(`Invalid updater asset URL ${url}: ${error.message}`);
  }
}

function findAssetByName(assets, name) {
  const asset = assets.find((candidate) => candidate.name === name);
  if (!asset) {
    throw new Error(`Release asset ${name} is missing`);
  }
  return asset;
}

function buildSignatureAssetMap(assets, feed) {
  const map = new Map();
  for (const key of REQUIRED_PLATFORMS) {
    const item = feed.platforms?.[key];
    if (!item?.url || !item?.signature) {
      throw new Error(`latest.json missing ${key} url/signature`);
    }
    const assetName = filenameFromUrl(item.url);
    if (!assetName) {
      throw new Error(`latest.json ${key} URL does not include a filename`);
    }
    map.set(key, findAssetByName(assets, `${assetName}.sig`));
  }
  return map;
}

function readUpdaterPublicKey() {
  const config = JSON.parse(fs.readFileSync(TAURI_CONFIG_PATH, "utf8"));
  const encoded = config.plugins?.updater?.pubkey;
  if (!encoded) {
    throw new Error(`Missing updater pubkey in ${TAURI_CONFIG_PATH}`);
  }
  const decoded = Buffer.from(encoded, "base64").toString("utf8");
  const lines = decoded.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  const publicKeyB64 = lines[1] || encoded;
  const publicKeyBin = Buffer.from(publicKeyB64, "base64");
  if (publicKeyBin.length !== 42) {
    throw new Error("Updater public key has an invalid minisign length");
  }
  const algorithm = publicKeyBin.subarray(0, 2);
  if (!((algorithm[0] === 0x45 && algorithm[1] === 0x64) || (algorithm[0] === 0x45 && algorithm[1] === 0x44))) {
    throw new Error("Updater public key uses an unsupported minisign algorithm");
  }
  return {
    keyId: publicKeyBin.subarray(2, 10),
    publicKey: crypto.createPublicKey({
      key: Buffer.concat([ED25519_SPKI_PREFIX, publicKeyBin.subarray(10, 42)]),
      format: "der",
      type: "spki",
    }),
  };
}

function parseMinisignSignature(signatureB64) {
  const text = decodeLatestSignature(signatureB64);
  const lines = text.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  if (lines.length < 4) {
    throw new Error("Minisign signature has too few lines");
  }
  const signatureBin = Buffer.from(lines[1], "base64");
  const globalSignature = Buffer.from(lines[3], "base64");
  if (signatureBin.length !== 74) {
    throw new Error("Minisign signature payload has an invalid length");
  }
  if (globalSignature.length !== 64) {
    throw new Error("Minisign global signature has an invalid length");
  }
  if (!lines[2].startsWith("trusted comment: ")) {
    throw new Error("Minisign signature is missing a trusted comment");
  }

  const algorithm = signatureBin.subarray(0, 2);
  const isPrehashed = algorithm[0] === 0x45 && algorithm[1] === 0x44;
  const isLegacy = algorithm[0] === 0x45 && algorithm[1] === 0x64;
  if (!isPrehashed && !isLegacy) {
    throw new Error("Minisign signature uses an unsupported algorithm");
  }

  return {
    isPrehashed,
    keyId: signatureBin.subarray(2, 10),
    signature: signatureBin.subarray(10, 74),
    trustedComment: lines[2].slice("trusted comment: ".length),
    globalSignature,
  };
}

function verifyMinisignSignature({ artifact, signatureB64, publicKey, label }) {
  const signature = parseMinisignSignature(signatureB64);
  if (!signature.keyId.equals(publicKey.keyId)) {
    throw new Error(`${label} signature key id does not match the configured updater public key`);
  }
  const message = signature.isPrehashed
    ? crypto.createHash("blake2b512").update(artifact).digest()
    : artifact;
  if (!crypto.verify(null, message, publicKey.publicKey, signature.signature)) {
    throw new Error(`${label} signature does not verify the updater artifact`);
  }
  const globalMessage = Buffer.concat([signature.signature, Buffer.from(signature.trustedComment, "utf8")]);
  if (!crypto.verify(null, globalMessage, publicKey.publicKey, signature.globalSignature)) {
    throw new Error(`${label} trusted comment signature is invalid`);
  }
}

async function main() {
  const { releasePath, latestPath, markerPath } = parseArgs();
  const token = process.env.GITHUB_TOKEN;
  if (!token) {
    throw new Error("GITHUB_TOKEN is required");
  }

  const release = JSON.parse(fs.readFileSync(releasePath, "utf8"));
  const feed = JSON.parse(fs.readFileSync(latestPath, "utf8"));
  const assets = release.assets || [];

  const latestAsset = assets.find((asset) => asset.name === "latest.json");
  if (!latestAsset) throw new Error("Release asset latest.json is missing");

  const signatureAssets = buildSignatureAssetMap(assets, feed);
  const signatureCache = new Map();
  const artifactCache = new Map();
  const publicKey = readUpdaterPublicKey();

  async function expectedSignatureForPlatform(key) {
    const asset = signatureAssets.get(key);
    if (!asset) throw new Error(`No signature asset mapped for ${key}`);
    if (!signatureCache.has(asset.id)) {
      signatureCache.set(asset.id, normalizeSignatureAsset(await downloadText(asset, token)));
    }
    return signatureCache.get(asset.id);
  }

  async function artifactForPlatform(key) {
    const item = feed.platforms?.[key];
    const assetName = filenameFromUrl(item.url);
    const asset = findAssetByName(assets, assetName);
    if (!artifactCache.has(asset.id)) {
      artifactCache.set(asset.id, downloadAsset(asset, token));
    }
    return artifactCache.get(asset.id);
  }

  let patched = 0;
  for (const key of REQUIRED_PLATFORMS) {
    const item = feed.platforms?.[key];
    if (!item?.url || !item?.signature) {
      throw new Error(`latest.json missing ${key} url/signature`);
    }
    decodeLatestSignature(item.signature);

    const expected = await expectedSignatureForPlatform(key);
    verifyMinisignSignature({
      artifact: await artifactForPlatform(key),
      signatureB64: expected,
      publicKey,
      label: key,
    });
    if (item.signature.trim() !== expected.trim()) {
      console.log(`Patching latest.json [${key}]`);
      console.log(`  old: ${displayRawSigLine(item.signature)}`);
      console.log(`  new: ${displayRawSigLine(expected)}`);
      item.signature = expected;
      patched++;
    }
  }

  if (patched > 0) {
    fs.writeFileSync(latestPath, JSON.stringify(feed, null, 2) + "\n");
    fs.writeFileSync(markerPath, "1");
    console.log(`Patched ${patched} platform signature(s); latest.json must be re-uploaded.`);
  } else {
    if (fs.existsSync(markerPath)) fs.rmSync(markerPath);
    console.log("latest.json signatures already match release .sig assets.");
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});

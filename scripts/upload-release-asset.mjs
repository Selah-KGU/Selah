#!/usr/bin/env node
// Replace one GitHub Release asset using release metadata from the GitHub API.

import fs from "node:fs";
import https from "node:https";

function parseArgs() {
  const args = process.argv.slice(2);
  const parsed = {
    releasePath: "release.json",
    filePath: null,
    name: null,
    tag: null,
    requireDraft: false,
    expectedReleaseId: null,
  };
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === "--release") {
      parsed.releasePath = args[++i];
    } else if (arg === "--file") {
      parsed.filePath = args[++i];
    } else if (arg === "--name") {
      parsed.name = args[++i];
    } else if (arg === "--tag") {
      parsed.tag = args[++i];
    } else if (arg === "--require-draft") {
      parsed.requireDraft = true;
    } else if (arg === "--expected-release-id") {
      parsed.expectedReleaseId = args[++i];
    } else {
      throw new Error(`Unexpected argument: ${arg}`);
    }
  }
  if (!parsed.filePath) throw new Error("--file is required");
  if (!parsed.name) throw new Error("--name is required");
  return parsed;
}

async function apiRequest({ method, hostname = "api.github.com", path, headers = {}, body = null }) {
  return new Promise((resolve, reject) => {
    const request = https.request(
      {
        hostname,
        path,
        method,
        headers,
      },
      (response) => {
        let text = "";
        response.setEncoding("utf8");
        response.on("data", (chunk) => {
          text += chunk;
        });
        response.on("end", () => resolve({ status: response.statusCode, body: text }));
        response.on("error", reject);
      },
    );
    request.on("error", reject);
    if (body) request.write(body);
    request.end();
  });
}

async function fetchReleaseByTag(repo, token, tag) {
  const result = await apiRequest({
    method: "GET",
    path: `/repos/${repo}/releases/tags/${encodeURIComponent(tag)}`,
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "User-Agent": "selah-ci",
      "X-GitHub-Api-Version": "2022-11-28",
    },
  });
  if (result.status !== 200) {
    throw new Error(`Failed to refresh release ${tag}: ${result.status} ${result.body}`);
  }
  return JSON.parse(result.body);
}

async function main() {
  const { releasePath, filePath, name, tag, requireDraft, expectedReleaseId } = parseArgs();
  const token = process.env.GITHUB_TOKEN;
  const repo = process.env.GITHUB_REPOSITORY;
  if (!token || !repo) {
    throw new Error("GITHUB_TOKEN and GITHUB_REPOSITORY are required");
  }

  const release = tag
    ? await fetchReleaseByTag(repo, token, tag)
    : JSON.parse(fs.readFileSync(releasePath, "utf8"));
  if (expectedReleaseId && String(release.id) !== String(expectedReleaseId)) {
    throw new Error(
      `Refusing to replace ${name}: release id changed from ${expectedReleaseId} to ${release.id}`
    );
  }
  if (requireDraft && !release.draft) {
    throw new Error(`Refusing to replace ${name}: release ${release.tag_name || tag || release.id} is not a draft`);
  }
  const existing = (release.assets || []).find((asset) => asset.name === name);
  if (existing) {
    const deleteResult = await apiRequest({
      method: "DELETE",
      path: `/repos/${repo}/releases/assets/${existing.id}`,
      headers: {
        Accept: "application/vnd.github+json",
        Authorization: `Bearer ${token}`,
        "User-Agent": "selah-ci",
        "X-GitHub-Api-Version": "2022-11-28",
      },
    });
    if (deleteResult.status !== 204) {
      throw new Error(`Failed to delete ${name}: ${deleteResult.status} ${deleteResult.body}`);
    }
    console.log(`Deleted old ${name} from release`);
  }

  const content = fs.readFileSync(filePath);
  const base = release.upload_url.replace(/\{[^}]+\}/, "");
  const url = new URL(base);
  url.searchParams.set("name", name);
  const uploadResult = await apiRequest({
    method: "POST",
    hostname: url.hostname,
    path: url.pathname + url.search,
    body: content,
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "User-Agent": "selah-ci",
      "X-GitHub-Api-Version": "2022-11-28",
      "Content-Type": "application/octet-stream",
      "Content-Length": content.length,
    },
  });
  if (uploadResult.status !== 201) {
    throw new Error(`Failed to upload ${name}: ${uploadResult.status} ${uploadResult.body}`);
  }
  console.log(`Uploaded ${name}`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});

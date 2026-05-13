#!/usr/bin/env node
// Find a GitHub release by tag, including draft releases.

import fs from "node:fs";
import https from "node:https";

function parseArgs() {
  const args = process.argv.slice(2);
  const parsed = {
    tag: null,
    output: "release.json",
    required: false,
  };
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === "--tag") {
      parsed.tag = args[++i];
    } else if (arg === "--output") {
      parsed.output = args[++i];
    } else if (arg === "--required") {
      parsed.required = true;
    } else {
      throw new Error(`Unexpected argument: ${arg}`);
    }
  }
  if (!parsed.tag) throw new Error("--tag is required");
  return parsed;
}

async function apiRequest({ path, token }) {
  return new Promise((resolve, reject) => {
    const request = https.request(
      {
        hostname: "api.github.com",
        path,
        method: "GET",
        headers: {
          Accept: "application/vnd.github+json",
          Authorization: `Bearer ${token}`,
          "User-Agent": "selah-ci",
          "X-GitHub-Api-Version": "2022-11-28",
        },
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
    request.end();
  });
}

async function findReleaseByTag(repo, token, tag) {
  const visible = [];
  for (let page = 1; page <= 5; page++) {
    const result = await apiRequest({
      token,
      path: `/repos/${repo}/releases?per_page=100&page=${page}`,
    });
    if (result.status !== 200) {
      throw new Error(`Failed to list releases: ${result.status} ${result.body}`);
    }
    const releases = JSON.parse(result.body);
    if (!Array.isArray(releases) || releases.length === 0) break;
    visible.push(...releases.slice(0, 20 - visible.length));
    const release = releases.find((item) => item.tag_name === tag);
    if (release) return { release, visible };
    if (releases.length < 100) break;
  }
  return { release: null, visible };
}

async function main() {
  const { tag, output, required } = parseArgs();
  const token = process.env.GITHUB_TOKEN;
  const repo = process.env.GITHUB_REPOSITORY;
  if (!token || !repo) {
    throw new Error("GITHUB_TOKEN and GITHUB_REPOSITORY are required");
  }
  const { release, visible } = await findReleaseByTag(repo, token, tag);
  if (release) {
    fs.writeFileSync(output, JSON.stringify(release));
    return;
  }
  if (required) {
    const visibleTags = visible.map((item) => `${item.tag_name}${item.draft ? " (draft)" : ""}`).join(", ");
    throw new Error(`Release ${tag} was not found. Visible releases: ${visibleTags || "(none)"}`);
  }
  process.exitCode = 2;
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});

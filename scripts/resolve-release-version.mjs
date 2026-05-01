#!/usr/bin/env node
// Resolve the next Selah release version while keeping the patch segment <= 999.
//
// Patch algorithm:
// - Split the UTC year into 36 ten-day-ish periods: early/mid/late for each month.
// - Each period owns 5 patch slots, so the date floor only grows to 176..180 in December.
// - If the highest vMAJOR.MINOR.* release is still a draft, reuse it.
// - If the highest matching release is published, use max(date floor, highest + 1).
//
// This preserves a visible date relationship without leaving semver's 3-part
// shape, while keeping the patch trend intentionally slow.

import fs from "node:fs";

const PACKAGE_PATH = "package.json";
const MAX_PATCH = 999;
const PERIOD_SLOT_SIZE = 5;

function readBase() {
  const pkg = JSON.parse(fs.readFileSync(PACKAGE_PATH, "utf8"));
  return pkg.version.split(".").slice(0, 2).join(".");
}

function parseArgs() {
  const args = process.argv.slice(2);
  const parsed = {
    base: null,
    github: false,
    json: false,
  };
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === "--base") {
      parsed.base = args[++i];
    } else if (arg === "--github") {
      parsed.github = true;
    } else if (arg === "--json") {
      parsed.json = true;
    } else {
      console.error(`Unexpected argument: ${arg}`);
      process.exit(1);
    }
  }
  return parsed;
}

function dateFloorPatch(now = new Date()) {
  const month = now.getUTCMonth();
  const day = now.getUTCDate();
  const monthPeriod = day <= 10 ? 0 : day <= 20 ? 1 : 2;
  return (month * 3 + monthPeriod) * PERIOD_SLOT_SIZE + 1;
}

function releasePattern(base) {
  const escaped = base.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`^v${escaped}\\.(\\d{1,3})$`);
}

async function githubJson(url, token) {
  const response = await fetch(url, {
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": "2022-11-28",
    },
  });
  if (!response.ok) {
    throw new Error(`GitHub API ${response.status} for ${url}: ${await response.text()}`);
  }
  return response.json();
}

async function listMatchingReleases(base) {
  const repo = process.env.GITHUB_REPOSITORY;
  const token = process.env.GITHUB_TOKEN;
  if (!repo || !token) {
    throw new Error("GITHUB_REPOSITORY and GITHUB_TOKEN are required with --github");
  }

  const pattern = releasePattern(base);
  const matches = [];
  for (let page = 1; ; page++) {
    const url = `https://api.github.com/repos/${repo}/releases?per_page=100&page=${page}`;
    const releases = await githubJson(url, token);
    if (!Array.isArray(releases) || releases.length === 0) break;
    for (const release of releases) {
      const match = pattern.exec(release.tag_name || "");
      if (!match) continue;
      const patch = Number(match[1]);
      if (!Number.isInteger(patch) || patch < 1 || patch > MAX_PATCH) continue;
      matches.push({
        patch,
        draft: Boolean(release.draft),
        tag: release.tag_name,
        url: release.html_url || "",
      });
    }
    if (releases.length < 100) break;
  }
  return matches.sort((a, b) => b.patch - a.patch);
}

async function resolveVersion({ base, github }) {
  if (!/^\d+\.\d+$/.test(base)) {
    throw new Error(`Invalid major.minor: ${base}`);
  }

  const floor = dateFloorPatch();
  let patch = floor;
  let reason = "date-floor";
  let highest = null;

  if (github) {
    const releases = await listMatchingReleases(base);
    highest = releases[0] || null;
    if (highest?.draft) {
      patch = highest.patch;
      reason = "reuse-draft";
    } else if (highest) {
      patch = Math.max(floor, highest.patch + 1);
      reason = highest.patch >= floor ? "published-increment" : "date-floor-after-published";
    }
  }

  if (patch > MAX_PATCH) {
    throw new Error(
      `No patch slot remains for ${base}.999. Bump MAJOR.MINOR before publishing another release.`
    );
  }

  return {
    version: `${base}.${patch}`,
    tag: `v${base}.${patch}`,
    base,
    patch,
    dateFloor: floor,
    reason,
    highest,
  };
}

const args = parseArgs();
try {
  const result = await resolveVersion({
    base: args.base ?? readBase(),
    github: args.github,
  });
  if (args.json) {
    console.log(JSON.stringify(result));
  } else {
    console.log(result.version);
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}

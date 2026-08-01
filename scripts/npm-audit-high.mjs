#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

const projectDir = resolve(process.argv[2] ?? ".");
const sourceDir = join(projectDir, "src");
const rscMarkers = [
  "react-server",
  "RSCHydratedRouter",
  "RSCStaticRouter",
  "createCallServer",
  "matchRSCServerRequest",
  "unstable_RSC",
  "unstable_createCallServer",
];

const acceptedAdvisories = new Map([
  [
    "https://github.com/advisories/GHSA-qwww-vcr4-c8h2",
    {
      package: "react-router",
      owner: "z-Bot maintainers",
      expires: "2026-10-31",
      reason:
        "React Router RSC action handling is not exposed by the Vite client-only dashboard; the source tree is checked for RSC markers before this exception is accepted.",
    },
  ],
]);

const audit = spawnSync("npm", ["audit", "--audit-level=high", "--json"], {
  cwd: projectDir,
  encoding: "utf8",
});

if (audit.error) {
  console.error(`Failed to run npm audit: ${audit.error.message}`);
  process.exit(1);
}

if (!audit.stdout.trim()) {
  process.stderr.write(audit.stderr);
  process.exit(audit.status ?? 1);
}

let report;
try {
  report = JSON.parse(audit.stdout);
} catch (error) {
  process.stderr.write(audit.stdout);
  process.stderr.write(audit.stderr);
  process.stderr.write(`Failed to parse npm audit JSON: ${error.message}\n`);
  process.exit(1);
}

if (report.error || !report.vulnerabilities || !report.metadata?.vulnerabilities) {
  console.error("npm audit did not return a valid vulnerability report:");
  console.error(JSON.stringify(report.error ?? report, null, 2));
  process.exit(1);
}

const today = new Date().toISOString().slice(0, 10);
for (const [url, exception] of acceptedAdvisories) {
  if (today > exception.expires) {
    console.error(`Expired npm audit exception: ${url} (expired ${exception.expires})`);
    process.exit(1);
  }
}

const vulnerabilities = report.vulnerabilities ?? {};
const unexpected = [];
const accepted = [];

function advisoryObjects(vulnerability) {
  return (vulnerability.via ?? []).filter((entry) => typeof entry === "object" && entry.url);
}

function viaNames(vulnerability) {
  return (vulnerability.via ?? []).filter((entry) => typeof entry === "string");
}

for (const [name, vulnerability] of Object.entries(vulnerabilities)) {
  if (!["high", "critical"].includes(vulnerability.severity)) {
    continue;
  }
  const advisories = advisoryObjects(vulnerability);
  if (advisories.length > 0) {
    const unaccepted = advisories.filter((advisory) => {
      const acceptedAdvisory = acceptedAdvisories.get(advisory.url);
      return !acceptedAdvisory || acceptedAdvisory.package !== advisory.name;
    });
    if (unaccepted.length > 0) {
      unexpected.push({ name, advisories: unaccepted });
    } else {
      accepted.push({ name, advisories });
    }
    continue;
  }

  const unresolvedVia = viaNames(vulnerability).filter((viaName) => {
    const via = vulnerabilities[viaName];
    return advisoryObjects(via).some((advisory) => {
      const acceptedAdvisory = acceptedAdvisories.get(advisory.url);
      return !acceptedAdvisory || acceptedAdvisory.package !== advisory.name;
    });
  });
  if (unresolvedVia.length > 0) {
    unexpected.push({ name, via: unresolvedVia });
  } else if (viaNames(vulnerability).length > 0) {
    accepted.push({ name, via: viaNames(vulnerability) });
  }
}

if (accepted.length > 0 && sourceUsesReactRouterRsc()) {
  console.error("React Router RSC audit exception is not valid because the UI source contains RSC markers.");
  process.exit(1);
}

if (unexpected.length > 0) {
  console.error("Unexpected npm audit high+ vulnerabilities:");
  console.error(JSON.stringify(unexpected, null, 2));
  process.exit(1);
}

if ((report.metadata?.vulnerabilities?.total ?? 0) > 0) {
  console.warn("Accepted npm audit advisory exception(s):");
  for (const [url, acceptedAdvisory] of acceptedAdvisories) {
    console.warn(`- ${acceptedAdvisory.package}: ${url}`);
    console.warn(`  Owner: ${acceptedAdvisory.owner}; expires: ${acceptedAdvisory.expires}`);
    console.warn(`  ${acceptedAdvisory.reason}`);
  }
}

process.exit(0);

function sourceUsesReactRouterRsc() {
  for (const file of walk(sourceDir)) {
    if (!/\.(?:[cm]?[jt]sx?)$/.test(file)) {
      continue;
    }
    const text = readFileSync(file, "utf8");
    if (rscMarkers.some((marker) => text.includes(marker))) {
      return true;
    }
  }
  return false;
}

function* walk(directory) {
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    const stat = statSync(path);
    if (stat.isDirectory()) {
      yield* walk(path);
    } else if (stat.isFile()) {
      yield path;
    }
  }
}

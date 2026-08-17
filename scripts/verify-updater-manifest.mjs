import { readFileSync } from "node:fs";

const [source] = process.argv.slice(2);
if (!source) {
  console.error(
    "usage: node scripts/verify-updater-manifest.mjs <latest.json url or path>",
  );
  process.exit(1);
}

async function loadManifest() {
  if (!source.startsWith("http")) return JSON.parse(readFileSync(source, "utf8"));

  const res = await fetch(source, { cache: "no-store" });
  if (!res.ok) {
    console.error(`Updater manifest unreachable: ${source} -> ${res.status}`);
    process.exit(1);
  }
  return res.json();
}

const manifest = await loadManifest();
const platforms = Object.entries(manifest.platforms ?? {});
if (platforms.length === 0) {
  console.error(`Updater manifest lists no platforms: ${source}`);
  process.exit(1);
}

const checks = await Promise.all(
  platforms.map(async ([key, { url, signature }]) => {
    const head = await fetch(url, { method: "HEAD", redirect: "follow" });
    return {
      key,
      url,
      status: head.status,
      ok: head.ok && Boolean(signature),
      signed: Boolean(signature),
    };
  }),
);

checks.forEach(({ key, url, status, signed }) =>
  console.log(`${status} ${signed ? "signed" : "UNSIGNED"} ${key} ${url}`),
);

const broken = checks.filter((check) => !check.ok);
if (broken.length > 0) {
  console.error(
    `\n${broken.length} of ${checks.length} updater downloads are not installable — ` +
      `users would see the update banner and get nothing.`,
  );
  process.exit(1);
}

console.log(`\nAll ${checks.length} updater downloads reachable and signed.`);

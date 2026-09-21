import { readFileSync } from "node:fs";

const [source] = process.argv.slice(2);
if (!source) {
  console.error(
    "usage: node scripts/verify-updater-manifest.mjs <latest.json url or path>",
  );
  process.exit(1);
}

async function loadManifest() {
  if (!source.startsWith("http"))
    return JSON.parse(readFileSync(source, "utf8"));

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

// A signature that is merely present proves nothing: a rebuilt release can
// leave the manifest holding the previous build's signatures while serving the
// new binaries, and every updater then refuses the download. Compare what the
// manifest claims with the .sig published beside the artifact.
const checks = await Promise.all(
  platforms.map(async ([key, { url, signature }]) => {
    const head = await fetch(url, { method: "HEAD", redirect: "follow" });
    const sigRes = await fetch(`${url}.sig`, { redirect: "follow" });
    const published = sigRes.ok ? (await sigRes.text()).trim() : "";
    const matches = Boolean(signature) && published === signature.trim();
    return {
      key,
      url,
      status: head.status,
      ok: head.ok && matches,
      signed: Boolean(signature),
      matches,
      published: Boolean(published),
    };
  }),
);

checks.forEach(({ key, url, status, signed, published, matches }) =>
  console.log(
    `${status} ${signed ? "signed" : "UNSIGNED"} ` +
      `${matches ? "signature matches the artifact" : published ? "SIGNATURE IS NOT THE ONE BESIDE THE ARTIFACT" : "NO PUBLISHED .sig"} ` +
      `${key} ${url}`,
  ),
);

const broken = checks.filter((check) => !check.ok);
if (broken.length > 0) {
  console.error(
    `\n${broken.length} of ${checks.length} updater downloads are not installable — ` +
      `users would see the update banner and get a failure.`,
  );
  process.exit(1);
}

console.log(
  `\nAll ${checks.length} updater downloads reachable, and signed by the .sig beside them.`,
);

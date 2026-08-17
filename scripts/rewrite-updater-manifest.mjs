import { readFileSync, writeFileSync } from "node:fs";

const OS_DIRS = { darwin: "macos", windows: "windows", linux: "linux" };

export function cdnUrl(platformKey, assetUrl, base) {
  const dir = OS_DIRS[platformKey.split("-")[0]];
  if (!dir) throw new Error(`Unknown platform key in manifest: ${platformKey}`);
  const filename = assetUrl.split("/").pop();
  if (!filename) throw new Error(`Manifest URL has no filename: ${assetUrl}`);
  return `${base.replace(/\/+$/, "")}/${dir}/${filename}`;
}

export function rewriteManifest(manifest, base) {
  const platforms = manifest.platforms;
  if (!platforms || Object.keys(platforms).length === 0)
    throw new Error("Manifest has no platforms — refusing to publish it");

  return {
    ...manifest,
    platforms: Object.fromEntries(
      Object.entries(platforms).map(([key, value]) => [
        key,
        { ...value, url: cdnUrl(key, value.url, base) },
      ]),
    ),
  };
}

const [input, base, output = input] = process.argv.slice(2);
if (input && base) {
  const manifest = rewriteManifest(
    JSON.parse(readFileSync(input, "utf8")),
    base,
  );
  writeFileSync(output, JSON.stringify(manifest, null, 2) + "\n");
  console.log(
    Object.entries(manifest.platforms)
      .map(([key, value]) => `${key} -> ${value.url}`)
      .join("\n"),
  );
}

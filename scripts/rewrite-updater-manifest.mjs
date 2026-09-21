import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const OS_DIRS = { darwin: "macos", windows: "windows", linux: "linux" };

export function cdnUrl(platformKey, assetUrl, base) {
  const dir = OS_DIRS[platformKey.split("-")[0]];
  if (!dir) throw new Error(`Unknown platform key in manifest: ${platformKey}`);
  const filename = assetUrl.split("/").pop();
  if (!filename) throw new Error(`Manifest URL has no filename: ${assetUrl}`);
  return `${base.replace(/\/+$/, "")}/${dir}/${filename}`;
}

// The signature comes from the .sig file sitting beside the artifact, never
// from the manifest we were handed: a rebuilt release keeps the first build's
// latest.json while every binary and .sig is replaced, and the updater then
// refuses each download with "signature verification failed".
export function rewriteManifest(manifest, base, readSignature) {
  const platforms = manifest.platforms;
  if (!platforms || Object.keys(platforms).length === 0)
    throw new Error("Manifest has no platforms — refusing to publish it");

  return {
    ...manifest,
    platforms: Object.fromEntries(
      Object.entries(platforms).map(([key, value]) => {
        const filename = value.url.split("/").pop();
        const signature = readSignature(filename);
        if (!signature)
          throw new Error(
            `No ${filename}.sig beside the artifact — refusing to publish a manifest nothing can verify`,
          );
        return [
          key,
          { ...value, url: cdnUrl(key, value.url, base), signature },
        ];
      }),
    ),
  };
}

export function signatureReader(dir) {
  return (filename) => {
    const sig = join(dir, `${filename}.sig`);
    return existsSync(sig) ? readFileSync(sig, "utf8").trim() : "";
  };
}

const [input, base, output = input] = process.argv.slice(2);
if (input && base) {
  const manifest = rewriteManifest(
    JSON.parse(readFileSync(input, "utf8")),
    base,
    signatureReader(dirname(input) || "."),
  );
  writeFileSync(output, JSON.stringify(manifest, null, 2) + "\n");
  console.log(
    Object.entries(manifest.platforms)
      .map(([key, value]) => `${key} -> ${value.url}`)
      .join("\n"),
  );
}

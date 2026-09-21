import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  cdnUrl,
  rewriteManifest,
  signatureReader,
} from "./rewrite-updater-manifest.mjs";

const BASE = "https://downloads.aircast.one/flasher/v0.1.1";

const SIGS = {
  "Aircast.Flasher_universal.app.tar.gz": "sig-mac",
  "Aircast.Flasher_0.1.1_x64-setup.exe": "sig-win",
  "Aircast.Flasher_0.1.1_amd64.AppImage": "sig-linux",
};
const readSig = (filename) => SIGS[filename] ?? "";

const GITHUB_MANIFEST = {
  version: "0.1.1",
  notes: "notes",
  platforms: {
    "darwin-aarch64": {
      signature: "sig-mac",
      url: "https://github.com/aircast-one/aircast-flasher/releases/download/v0.1.1/Aircast.Flasher_universal.app.tar.gz",
    },
    "windows-x86_64-nsis": {
      signature: "sig-win",
      url: "https://github.com/aircast-one/aircast-flasher/releases/download/v0.1.1/Aircast.Flasher_0.1.1_x64-setup.exe",
    },
    "linux-x86_64-appimage": {
      signature: "sig-linux",
      url: "https://github.com/aircast-one/aircast-flasher/releases/download/v0.1.1/Aircast.Flasher_0.1.1_amd64.AppImage",
    },
  },
};

test("sends each platform to its own directory on the downloads host", () => {
  const { platforms } = rewriteManifest(GITHUB_MANIFEST, BASE, readSig);

  assert.equal(
    platforms["darwin-aarch64"].url,
    `${BASE}/macos/Aircast.Flasher_universal.app.tar.gz`,
  );
  assert.equal(
    platforms["windows-x86_64-nsis"].url,
    `${BASE}/windows/Aircast.Flasher_0.1.1_x64-setup.exe`,
  );
  assert.equal(
    platforms["linux-x86_64-appimage"].url,
    `${BASE}/linux/Aircast.Flasher_0.1.1_amd64.AppImage`,
  );
});

test("leaves no github.com download in the published manifest", () => {
  const rewritten = rewriteManifest(GITHUB_MANIFEST, BASE, readSig);
  assert.equal(JSON.stringify(rewritten).includes("github.com"), false);
});

test("takes each signature from the .sig beside the artifact, not from the manifest", () => {
  const stale = {
    ...GITHUB_MANIFEST,
    platforms: Object.fromEntries(
      Object.entries(GITHUB_MANIFEST.platforms).map(([key, value]) => [
        key,
        { ...value, signature: "signature-of-an-earlier-build" },
      ]),
    ),
  };

  const { platforms } = rewriteManifest(stale, BASE, readSig);

  assert.equal(platforms["darwin-aarch64"].signature, "sig-mac");
  assert.equal(platforms["windows-x86_64-nsis"].signature, "sig-win");
  assert.equal(platforms["linux-x86_64-appimage"].signature, "sig-linux");
});

test("refuses to publish a platform whose artifact has no signature", () => {
  assert.throws(
    () => rewriteManifest(GITHUB_MANIFEST, BASE, () => ""),
    /refusing to publish a manifest nothing can verify/,
  );
});

test("reads the signature off disk, trimmed, and reports an absent one as empty", () => {
  const dir = mkdtempSync(join(tmpdir(), "sig-"));
  writeFileSync(join(dir, "app.tar.gz.sig"), "  base64-signature\n");
  const read = signatureReader(dir);

  assert.equal(read("app.tar.gz"), "base64-signature");
  assert.equal(read("never-built.exe"), "");
});

test("keeps version and notes", () => {
  const rewritten = rewriteManifest(GITHUB_MANIFEST, BASE, readSig);
  assert.equal(rewritten.version, "0.1.1");
  assert.equal(rewritten.notes, "notes");
});

test("tolerates a trailing slash on the base", () => {
  const { platforms } = rewriteManifest(GITHUB_MANIFEST, `${BASE}/`, readSig);
  assert.equal(
    platforms["darwin-aarch64"].url,
    `${BASE}/macos/Aircast.Flasher_universal.app.tar.gz`,
  );
});

test("refuses a manifest with no platforms rather than publishing a dead one", () => {
  assert.throws(
    () => rewriteManifest({ version: "0.1.1" }, BASE, readSig),
    /no platforms/,
  );
  assert.throws(
    () => rewriteManifest({ platforms: {} }, BASE, readSig),
    /no platforms/,
  );
});

test("refuses a platform key it cannot place", () => {
  assert.throws(
    () => cdnUrl("solaris-sparc", "https://example.com/app.tar.gz", BASE),
    /Unknown platform key/,
  );
});

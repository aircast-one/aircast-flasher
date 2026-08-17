import test from "node:test";
import assert from "node:assert/strict";

import { cdnUrl, rewriteManifest } from "./rewrite-updater-manifest.mjs";

const BASE = "https://downloads.aircast.one/flasher/v0.1.1";

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
  const { platforms } = rewriteManifest(GITHUB_MANIFEST, BASE);

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
  const rewritten = rewriteManifest(GITHUB_MANIFEST, BASE);
  assert.equal(JSON.stringify(rewritten).includes("github.com"), false);
});

test("keeps the signatures, which sign bytes rather than locations", () => {
  const { platforms } = rewriteManifest(GITHUB_MANIFEST, BASE);
  assert.equal(platforms["darwin-aarch64"].signature, "sig-mac");
  assert.equal(platforms["windows-x86_64-nsis"].signature, "sig-win");
});

test("keeps version and notes", () => {
  const rewritten = rewriteManifest(GITHUB_MANIFEST, BASE);
  assert.equal(rewritten.version, "0.1.1");
  assert.equal(rewritten.notes, "notes");
});

test("tolerates a trailing slash on the base", () => {
  const { platforms } = rewriteManifest(GITHUB_MANIFEST, `${BASE}/`);
  assert.equal(
    platforms["darwin-aarch64"].url,
    `${BASE}/macos/Aircast.Flasher_universal.app.tar.gz`,
  );
});

test("refuses a manifest with no platforms rather than publishing a dead one", () => {
  assert.throws(() => rewriteManifest({ version: "0.1.1" }, BASE), /no platforms/);
  assert.throws(() => rewriteManifest({ platforms: {} }, BASE), /no platforms/);
});

test("refuses a platform key it cannot place", () => {
  assert.throws(
    () => cdnUrl("solaris-sparc", "https://example.com/app.tar.gz", BASE),
    /Unknown platform key/,
  );
});

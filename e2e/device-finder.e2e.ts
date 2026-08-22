import http from "node:http";
import type { AddressInfo } from "node:net";

import { browser, expect } from "@wdio/globals";

const ATTACH_TIMEOUT = 30_000;

type TauriGlobal = {
  core: {
    invoke: (cmd: string, args: Record<string, unknown>) => Promise<boolean>;
  };
};

async function attachToAppWindow() {
  await browser.waitUntil(
    async () => {
      const handles = await browser.getWindowHandles();
      for (const handle of handles) {
        await browser.switchToWindow(handle);
        const hasApp = await browser
          .execute(() => Boolean(document.getElementById("root")))
          .catch(() => false);
        if (hasApp) return true;
      }
      return false;
    },
    { timeout: ATTACH_TIMEOUT, timeoutMsg: "no window held the app" },
  );
}

function probeFromApp(url: string): Promise<boolean | string> {
  return browser.executeAsync<boolean | string, [string]>(
    (target, done) => {
      const tauri = (window as { __TAURI__?: TauriGlobal }).__TAURI__;
      if (!tauri) return done("no-global-tauri");
      tauri.core
        .invoke("probe_device", { url: target })
        .then(done, (e: unknown) => done(`invoke failed: ${String(e)}`));
    },
    url,
  );
}

describe("device probing", () => {
  before(async () => {
    await attachToAppWindow();
  });

  it("finds a device whose /healthz answers", async () => {
    const server = http.createServer((req, res) => {
      res.writeHead(req.url === "/healthz" ? 200 : 404);
      res.end();
    });
    await new Promise<void>((resolve) => {
      server.listen(0, "127.0.0.1", resolve);
    });
    const { port } = server.address() as AddressInfo;
    try {
      expect(await probeFromApp(`http://127.0.0.1:${port}`)).toBe(true);
    } finally {
      server.close();
    }
  });

  it("finds nothing behind a closed port", async () => {
    expect(await probeFromApp("http://127.0.0.1:1")).toBe(false);
  });
});

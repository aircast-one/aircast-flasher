import http from "node:http";
import type { AddressInfo } from "node:net";

import { browser, expect } from "@wdio/globals";

const ATTACH_TIMEOUT = 30_000;

type TauriGlobal = {
  core: {
    invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown>;
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

function invokeFromApp(
  command: string,
  url: string,
): Promise<boolean | string> {
  return browser.executeAsync<boolean | string, [string, string]>(
    (cmd, target, done) => {
      const tauri = (window as { __TAURI__?: TauriGlobal }).__TAURI__;
      if (!tauri) return done("no-global-tauri");
      tauri.core.invoke(cmd, { url: target }).then(
        (v) => done(typeof v === "object" ? JSON.stringify(v) : (v as boolean | string)),
        (e) => done(`invoke failed: ${String(e)}`),
      );
    },
    command,
    url,
  );
}

const probeFromApp = (url: string) => invokeFromApp("probe_device", url);
const loginFromApp = (url: string) => invokeFromApp("tailscale_login", url);
const statusFromApp = (url: string) => invokeFromApp("tailscale_status", url);

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

  it("returns the auth URL aircastd hands back for a headless login", async () => {
    const server = http.createServer((req, res) => {
      if (req.method !== "POST" || req.url !== "/api/tailscale/connection") {
        res.writeHead(404);
        return res.end();
      }
      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          status: "login started",
          authURL: "https://login.tailscale.com/a/abc123",
        }),
      );
    });
    await new Promise<void>((resolve) => {
      server.listen(0, "127.0.0.1", resolve);
    });
    const { port } = server.address() as AddressInfo;
    try {
      expect(await loginFromApp(`http://127.0.0.1:${port}`)).toBe(
        "https://login.tailscale.com/a/abc123",
      );
    } finally {
      server.close();
    }
  });

  it("reports a device that has joined a tailnet, with its name", async () => {
    const server = http.createServer((req, res) => {
      if (req.url !== "/api/tailscale") {
        res.writeHead(404);
        return res.end();
      }
      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          loggedIn: true,
          tailnet: "example.com",
          dnsName: "falcon-01.tail9c2f.ts.net.",
        }),
      );
    });
    await new Promise<void>((resolve) => {
      server.listen(0, "127.0.0.1", resolve);
    });
    const { port } = server.address() as AddressInfo;
    try {
      expect(await statusFromApp(`http://127.0.0.1:${port}`)).toBe(
        JSON.stringify({
          connected: true,
          name: "falcon-01.tail9c2f.ts.net",
        }),
      );
    } finally {
      server.close();
    }
  });

  it("reports a device that has not signed in yet", async () => {
    const server = http.createServer((_req, res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ loggedIn: false, backendState: "NeedsLogin" }));
    });
    await new Promise<void>((resolve) => {
      server.listen(0, "127.0.0.1", resolve);
    });
    const { port } = server.address() as AddressInfo;
    try {
      expect(await statusFromApp(`http://127.0.0.1:${port}`)).toBe(
        JSON.stringify({ connected: false, name: "" }),
      );
    } finally {
      server.close();
    }
  });

  it("always answers whether this computer can reach a tailnet", async () => {
    const shape = await browser.executeAsync<string, []>((done) => {
      const tauri = (window as { __TAURI__?: TauriGlobal }).__TAURI__;
      if (!tauri) return done("no-global-tauri");
      tauri.core.invoke("local_tailscale", {}).then(
        (v) => {
          const s = v as Record<string, unknown>;
          done(
            [
              typeof s.installed,
              typeof s.running,
              typeof s.name,
              typeof s.magicDnsSuffix,
              Array.isArray(s.peers) ? "array" : typeof s.peers,
            ].join(","),
          );
        },
        (e) => done(`invoke failed: ${String(e)}`),
      );
    });
    expect(shape).toBe("boolean,boolean,string,string,array");
  });

  it("returns an empty URL when the device is already connected", async () => {
    const server = http.createServer((_req, res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ status: "already connected" }));
    });
    await new Promise<void>((resolve) => {
      server.listen(0, "127.0.0.1", resolve);
    });
    const { port } = server.address() as AddressInfo;
    try {
      expect(await loginFromApp(`http://127.0.0.1:${port}`)).toBe("");
    } finally {
      server.close();
    }
  });
});

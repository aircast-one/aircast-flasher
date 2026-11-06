import fs from "node:fs";
import http from "node:http";
import type { AddressInfo } from "node:net";
import path from "node:path";

import { browser, expect } from "@wdio/globals";

import { e2eConfigDir } from "./settings-file";

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
  args: Record<string, unknown>,
): Promise<boolean | string | null> {
  return browser.executeAsync<
    boolean | string | null,
    [string, Record<string, unknown>]
  >(
    (cmd, cmdArgs, done) => {
      const tauri = (window as { __TAURI__?: TauriGlobal }).__TAURI__;
      if (!tauri) return done("no-global-tauri");
      tauri.core.invoke(cmd, cmdArgs).then(
        (v) =>
          done(
            v !== null && typeof v === "object"
              ? JSON.stringify(v)
              : (v as boolean | string | null),
          ),
        (e) => done(`invoke failed: ${String(e)}`),
      );
    },
    command,
    args,
  );
}

const probeFromApp = (url: string) => invokeFromApp("probe_device", { url });
const loginFromApp = (url: string) => invokeFromApp("tailscale_login", { url });
const statusFromApp = (url: string) =>
  invokeFromApp("tailscale_status", { url });

const BUNDLE = JSON.stringify({
  format: 1,
  hostname: "falcon",
  created: "2026-11-01T12:00:00Z",
  files: [
    {
      path: "/var/lib/aircastd/gimbal.json",
      mode: 384,
      data: "eyJ0aWx0IjoxfQ==",
    },
  ],
});

async function backupServer(
  status: number,
  body: string,
): Promise<{ url: string; close: () => void }> {
  const server = http.createServer((req, res) => {
    if (req.method !== "GET" || req.url !== "/api/backup") {
      res.writeHead(404);
      return res.end();
    }
    res.writeHead(status, {
      "content-type": "application/json",
      "content-disposition": 'attachment; filename="backup.json"',
    });
    res.end(body);
  });
  await new Promise<void>((resolve) => {
    server.listen(0, "127.0.0.1", resolve);
  });
  const { port } = server.address() as AddressInfo;
  return { url: `http://127.0.0.1:${port}`, close: () => server.close() };
}

describe("device probing", () => {
  before(async () => {
    await attachToAppWindow();
  });

  it("finds aircastd by its /healthz and reports the IP that answered", async () => {
    const server = http.createServer((req, res) => {
      res.writeHead(req.url === "/healthz" ? 200 : 404);
      res.end(req.url === "/healthz" ? "ok" : "");
    });
    await new Promise<void>((resolve) => {
      server.listen(0, "127.0.0.1", resolve);
    });
    const { port } = server.address() as AddressInfo;
    try {
      expect(await probeFromApp(`http://127.0.0.1:${port}`)).toBe("127.0.0.1");
    } finally {
      server.close();
    }
  });

  it("does not take a captive portal that redirects every page for a device", async () => {
    const server = http.createServer((req, res) => {
      if (req.url === "/") {
        res.writeHead(200, { "content-type": "text/html" });
        return res.end("<html>ExpressLRS</html>");
      }
      res.writeHead(302, { location: "/" });
      res.end();
    });
    await new Promise<void>((resolve) => {
      server.listen(0, "127.0.0.1", resolve);
    });
    const { port } = server.address() as AddressInfo;
    try {
      expect(await probeFromApp(`http://127.0.0.1:${port}`)).toBe(null);
    } finally {
      server.close();
    }
  });

  it("finds nothing behind a closed port", async () => {
    expect(await probeFromApp("http://127.0.0.1:1")).toBe(null);
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

describe("settings backups", () => {
  before(async () => {
    await attachToAppWindow();
  });

  it("keeps the backup a drone hands over as sent, owner-only, until deleted", async () => {
    const drone = await backupServer(200, BUNDLE);
    try {
      const saved = JSON.parse(
        String(await invokeFromApp("backup_device", { url: drone.url })),
      );
      expect(saved).toEqual({
        id: "falcon-20261101120000",
        hostname: "falcon",
        created: "2026-11-01T12:00:00Z",
        card: null,
      });
      const file = path.join(e2eConfigDir(), "backups", `${saved.id}.json`);
      expect(fs.readFileSync(file, "utf8")).toBe(BUNDLE);
      if (process.platform !== "win32") {
        expect(fs.statSync(file).mode & 0o777).toBe(0o600);
      }
      expect(
        JSON.parse(String(await invokeFromApp("list_backups", {}))),
      ).toContainEqual(saved);

      await invokeFromApp("delete_backup", { id: saved.id });
      expect(fs.existsSync(file)).toBe(false);
    } finally {
      drone.close();
    }
  });

  it("tells a drone without the endpoint to update aircastd", async () => {
    const drone = await backupServer(404, "");
    try {
      expect(await invokeFromApp("backup_device", { url: drone.url })).toContain(
        "update aircastd on the drone first",
      );
    } finally {
      drone.close();
    }
  });

  it("refuses a backup in a format it does not know", async () => {
    const drone = await backupServer(200, BUNDLE.replace('"format":1', '"format":2'));
    try {
      expect(await invokeFromApp("backup_device", { url: drone.url })).toContain(
        "format 2",
      );
      expect(
        JSON.parse(String(await invokeFromApp("list_backups", {}))),
      ).toEqual([]);
    } finally {
      drone.close();
    }
  });

  it("says when the drone can't be reached", async () => {
    expect(
      await invokeFromApp("backup_device", { url: "http://127.0.0.1:1" }),
    ).toContain("Couldn't reach http://127.0.0.1:1");
  });

  it("reads what a backup records about the card, splitting Aircast Cloud from your own key servers", async () => {
    const card = {
      fightMode: true,
      passwordLogin: false,
      encryption: {
        threshold: 1,
        tangUrls: [
          "https://tang.example.com",
          "https://api.aircast.one/v1/tang/3f2a",
        ],
      },
      controlServer: "https://hs.example.com",
      tailnetNode: true,
    };
    const drone = await backupServer(
      200,
      JSON.stringify({ ...JSON.parse(BUNDLE), card }),
    );
    try {
      const saved = JSON.parse(
        String(await invokeFromApp("backup_device", { url: drone.url })),
      );
      expect(saved.card).toEqual({
        ...card,
        encryption: {
          threshold: 1,
          tangUrls: ["https://tang.example.com"],
          cloud: true,
        },
      });
      await invokeFromApp("delete_backup", { id: saved.id });
    } finally {
      drone.close();
    }
  });

  it("keeps only the newest five backups of a drone", async () => {
    const days = [1, 2, 3, 4, 5, 6, 7].values();
    const server = http.createServer((_req, res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        BUNDLE.replace('"falcon"', '"kestrel"').replace(
          "2026-11-01",
          `2026-11-0${days.next().value}`,
        ),
      );
    });
    await new Promise<void>((resolve) => {
      server.listen(0, "127.0.0.1", resolve);
    });
    const { port } = server.address() as AddressInfo;
    try {
      await [1, 2, 3, 4, 5, 6, 7].reduce(
        (previous) =>
          previous.then(() =>
            invokeFromApp("backup_device", { url: `http://127.0.0.1:${port}` }),
          ),
        Promise.resolve<boolean | string | null>(true),
      );
      const kept = (
        JSON.parse(String(await invokeFromApp("list_backups", {}))) as {
          id: string;
        }[]
      ).map((b) => b.id);
      expect(kept).toEqual(
        [7, 6, 5, 4, 3].map((day) => `kestrel-2026110${day}120000`),
      );
      await Promise.all(
        kept.map((id) => invokeFromApp("delete_backup", { id })),
      );
    } finally {
      server.close();
    }
  });

  it("backs up a drone once at a time", async () => {
    const held: { res: http.ServerResponse | null; released: boolean } = {
      res: null,
      released: false,
    };
    const answer = (res: http.ServerResponse) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(BUNDLE);
    };
    const server = http.createServer((req, res) => {
      if (req.url === "/api/backup") {
        if (held.released) return answer(res);
        held.res = res;
        return;
      }
      held.released = true;
      res.writeHead(200);
      res.end();
      if (held.res) answer(held.res);
    });
    await new Promise<void>((resolve) => {
      server.listen(0, "127.0.0.1", resolve);
    });
    const { port } = server.address() as AddressInfo;
    try {
      const results = await browser.executeAsync<string[], [string]>(
        (url, done) => {
          const tauri = (window as { __TAURI__?: TauriGlobal }).__TAURI__;
          if (!tauri) return done(["no-global-tauri"]);
          const settle = (p: Promise<unknown>) =>
            p.then(
              (v) => `ok:${JSON.stringify(v)}`,
              (e) => `err:${String(e)}`,
            );
          const first = settle(tauri.core.invoke("backup_device", { url }));
          const second = settle(tauri.core.invoke("backup_device", { url }));
          Promise.race([first, second])
            .then(() => tauri.core.invoke("probe_device", { url }))
            .then(() => Promise.all([first, second]))
            .then(done);
        },
        `http://127.0.0.1:${port}`,
      );
      expect(results.filter((r) => r.startsWith("ok:"))).toHaveLength(1);
      expect(results.filter((r) => r.includes("already being backed up"))).toHaveLength(1);
      await invokeFromApp("delete_backup", { id: "falcon-20261101120000" });
    } finally {
      server.close();
    }
  });
});

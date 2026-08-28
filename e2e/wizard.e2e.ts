import fs from "node:fs";

import { browser, $, $$, expect } from "@wdio/globals";

import { settingsFile } from "./settings-file";

const HEADING_TIMEOUT = 30_000;
const CATALOG_TIMEOUT = 60_000;

async function heading(): Promise<string> {
  return $("h1")
    .getText()
    .catch(() => "");
}

/// What the webview actually holds, for when the app renders nothing at all —
/// re-importing an already-failed ES module re-throws, which surfaces a
/// load-time exception that no console is around to catch.
async function pageDiagnostics(): Promise<string> {
  const dump = await browser
    .execute(() => {
      const root = document.getElementById("root");
      return JSON.stringify({
        href: location.href,
        readyState: document.readyState,
        title: document.title,
        bodyChars: document.body ? document.body.innerHTML.length : -1,
        rootChars: root ? root.innerHTML.length : -1,
        scripts: Array.from(document.querySelectorAll("script")).map(
          (s) => (s as HTMLScriptElement).src || "inline",
        ),
        stylesheets: document.querySelectorAll("link[rel=stylesheet]").length,
        cryptoType: typeof crypto,
        getRandomValues: typeof crypto?.getRandomValues,
        isSecureContext: window.isSecureContext,
      });
    })
    .catch((e: unknown) => `execute failed: ${String(e)}`);

  const moduleError = await browser
    .executeAsync((done: (r: string) => void) => {
      const src = document
        .querySelector("script[type=module]")
        ?.getAttribute("src");
      if (!src) return done("no module script in the document");
      import(src)
        .then(() => done("module evaluated without throwing"))
        .catch((e: unknown) =>
          done(
            `module threw: ${String((e as Error)?.stack ?? (e as Error)?.message ?? e)}`,
          ),
        );
    })
    .catch((e: unknown) => `module probe failed: ${String(e)}`);

  return `page=${dump} ${moduleError}`;
}

async function waitForHeading(text: string) {
  await browser
    .waitUntil(async () => (await heading()) === text, {
      timeout: HEADING_TIMEOUT,
    })
    .catch(async () => {
      throw new Error(
        `heading never became "${text}" (was "${await heading()}"). ${await pageDiagnostics()}`,
      );
    });
}

async function onNetworkStep() {
  if ((await heading()) === "Network & access") return;
  const back = $('nav[aria-label="Setup steps"]').$("button*=Network & access");
  if (await back.isEnabled()) {
    await back.click();
  } else {
    const next = $("button=Next");
    await next.waitForEnabled({
      timeout: CATALOG_TIMEOUT,
      timeoutMsg:
        "no OS image became selectable — is the release catalog reachable?",
    });
    await next.click();
  }
  await waitForHeading("Network & access");
}

async function knownGoodNetworkState() {
  await onNetworkStep();
  await $("#hostname").setValue("falcon-01");
  await $("#ssid").setValue("test-net");
  await $("#password").clearValue();
}

/// The session does not necessarily start on the app: on Linux the first handle
/// is an about:blank window, which is why a bare switchToWindow(handles[0])
/// looked exactly like an app that renders nothing. Find the window that holds
/// the wizard, and switch to it explicitly — that also sets the service's
/// suppression flag, which stops it polling window state before every element
/// lookup (5s per command otherwise).
async function showsApp(): Promise<boolean> {
  return browser
    .execute(() => Boolean(document.getElementById("root")))
    .catch(() => false);
}

async function attachToAppWindow() {
  await browser.waitUntil(
    async () => {
      const handles = await browser.getWindowHandles();
      for (const handle of handles) {
        await browser.switchToWindow(handle);
        if (await showsApp()) return true;
      }
      return false;
    },
    { timeout: HEADING_TIMEOUT },
  ).catch(async () => {
    throw new Error(
      `no window held the app. ${await pageDiagnostics()}`,
    );
  });
}

describe("flasher wizard", () => {
  before(async () => {
    await attachToAppWindow();
    await waitForHeading("Operating system");
  });

  it("asks before any diagnostics leave the machine, and only once", async () => {
    const banner = $('[data-testid="consent-banner"]');
    await expect(banner).toBeDisplayed();

    await banner.$("button=No thanks").click();

    await expect(banner).not.toBeDisplayed();
    await browser.waitUntil(
      () =>
        fs.existsSync(settingsFile()) &&
        JSON.parse(fs.readFileSync(settingsFile(), "utf8")).telemetry === false,
      {
        timeout: 10_000,
        timeoutMsg:
          "the answer never reached settings.json — the prompt would return on every launch",
      },
    );
  });

  it("names steps in the sidebar the same way the pages do", async () => {
    const labels = await $$(
      'nav[aria-label="Setup steps"] button span:last-child',
    ).map((s) => s.getText());
    expect(labels).toEqual([
      "Operating system",
      "Network & access",
      "Storage",
      "Write",
    ]);
  });

  it("arrives with a hostname already filled in", async () => {
    await onNetworkStep();
    await expect($("#hostname")).not.toHaveValue("");
  });

  it("unblocks Next once the network is named", async () => {
    await onNetworkStep();
    await $("#ssid").clearValue();
    await expect($("button=Next")).toBeDisabled();
    await $("#ssid").setValue("test-net");
    await expect($("button=Next")).toBeEnabled();
  });

  it("says why it blocks when the hostname is cleared", async () => {
    await knownGoodNetworkState();
    await $("#hostname").clearValue();
    await expect($("#hostname-help")).toHaveText(
      "Enter a hostname to continue.",
    );
    await expect($("button=Next")).toBeDisabled();
  });

  it("keeps a cleared hostname cleared instead of refilling it", async () => {
    await knownGoodNetworkState();
    await $("#hostname").clearValue();
    await onNetworkStep();
    await expect($("#hostname")).toHaveValue("");
  });

  it("rejects an invalid hostname with the rule it broke", async () => {
    await knownGoodNetworkState();
    await $("#hostname").setValue("Falcon_01");
    await expect($("#hostname-help")).toHaveText(
      expect.stringContaining("lowercase letters, numbers, and hyphens"),
    );
    await expect($("button=Next")).toBeDisabled();
  });

  it("refuses to write a card with no network at all", async () => {
    await knownGoodNetworkState();
    await $("#ssid").clearValue();
    await expect($("#ssid-help")).toHaveText(
      expect.stringContaining("never comes online"),
    );
    await expect($("button=Next")).toBeDisabled();
  });

  it("lets a wired device skip WiFi on purpose", async () => {
    await knownGoodNetworkState();
    await $("#ssid").clearValue();
    await $("button*=Ethernet or cellular only").click();
    await expect($("button=Next")).toBeEnabled();

    await $("button*=Set up WiFi instead").click();
    await expect($("#ssid")).toBeDisplayed();
    await expect($("button=Next")).toBeDisabled();
  });

  it("refuses a WiFi password the device could never join with", async () => {
    await knownGoodNetworkState();
    await $("#password").setValue("short");
    await expect($("#password-help")).toHaveText(
      expect.stringContaining("8–63 characters"),
    );
    await expect($("button=Next")).toBeDisabled();

    await $("#password").setValue("longenough");
    await expect($("button=Next")).toBeEnabled();
    await $("#password").clearValue();
  });

  it("refuses a Headscale key with no control server", async () => {
    await knownGoodNetworkState();
    await $("button*=Remote access").click();
    await $("button*=self-hosted control server").click();
    await $("#auth-key").setValue("hskey-auth-abc123");
    await expect($("#control-server-error")).toHaveText(
      expect.stringContaining("Enter your control server URL"),
    );
    await expect($("button=Next")).toBeDisabled();

    await $("button=Use Tailscale").click();
    await expect($("button=Next")).toBeEnabled();
    await $("#auth-key").clearValue();
  });

  it("never prefills the stock device password", async () => {
    await knownGoodNetworkState();
    await $("button=Password").click();
    await expect($("#device-password")).toHaveValue("");
    await expect($("button=Next")).toBeEnabled();
    await $("button=SSH key").click();
  });

  it("shows the login the device will ship with, without being asked", async () => {
    await onNetworkStep();
    await expect($("button=SSH key")).toBeDisplayed();
    await expect($("button=Disabled")).toBeDisplayed();
  });

  it("persists settings to disk, on every platform", async () => {
    await knownGoodNetworkState();
    await browser.waitUntil(
      () =>
        fs.existsSync(settingsFile()) &&
        JSON.parse(fs.readFileSync(settingsFile(), "utf8")).hostname ===
          "falcon-01",
      {
        timeout: 10_000,
        timeoutMsg: `settings never reached ${settingsFile()} — the wizard would forget them on restart`,
      },
    );
  });

  it("shows what will be written before the destructive step", async () => {
    await knownGoodNetworkState();
    await $("button=Next").click();

    await waitForHeading("Storage");
    await expect($("h2=What will be written")).toBeDisplayed();
    const rows = $("dl");
    await expect(rows).toHaveText(expect.stringContaining("falcon-01.local"));
    await expect(rows).toHaveText(expect.stringContaining("test-net"));
    await expect(rows).toHaveText(expect.stringContaining("Target card"));
    await expect($("button=Flash SD Card")).toBeDisabled();
  });

  it("sends you back to the step that owns a setting you want to change", async () => {
    await knownGoodNetworkState();
    await $("button=Next").click();
    await waitForHeading("Storage");

    await $('button[aria-label="Change Image"]').click();
    await waitForHeading("Operating system");
  });
});

import { browser, $, $$, expect } from "@wdio/globals";

const RELOAD_TIMEOUT = 30_000;
const CATALOG_TIMEOUT = 60_000;

const STORAGE_KEYS = [
  "aircast.wifi.ssid",
  "aircast.wifi.password",
  "aircast.hostname",
  "aircast.tailscale.controlServer",
  "aircast.ssh.authorizedKey",
];

/// The operator's real settings, so the suite can put them back afterwards: the
/// debug binary shares its WebKit data store with the installed app. WebKitGTK
/// denies localStorage on the app origin (SecurityError), so treat storage as
/// best-effort — the wizard's own useLocalStorage swallows the same failure.
async function readStorage(): Promise<Record<string, string | null>> {
  const json = await browser.execute((keys: string) => {
    try {
      return JSON.stringify(
        Object.fromEntries(
          (JSON.parse(keys) as string[]).map((k) => [
            k,
            localStorage.getItem(k),
          ]),
        ),
      );
    } catch {
      return "{}";
    }
  }, JSON.stringify(STORAGE_KEYS));
  return JSON.parse(json) as Record<string, string | null>;
}

async function writeStorage(entries: Record<string, string | null>) {
  await browser.execute((json: string) => {
    try {
      localStorage.clear();
      Object.entries(JSON.parse(json) as Record<string, string | null>).forEach(
        ([k, v]) => v !== null && localStorage.setItem(k, v),
      );
    } catch {
      // no persistence on this platform; the wizard falls back to defaults
    }
  }, JSON.stringify(entries));
}

async function heading(): Promise<string> {
  return $("h1")
    .getText()
    .catch(() => "");
}

async function waitForHeading(text: string) {
  await browser.waitUntil(async () => (await heading()) === text, {
    timeout: RELOAD_TIMEOUT,
    timeoutMsg: `heading never became "${text}" (was "${await heading()}")`,
  });
}

async function reloadPristine() {
  await writeStorage({});
  await browser.execute(() => {
    Object.assign(window, { __reloading: true });
    setTimeout(() => location.reload(), 0);
  });
  await browser.waitUntil(
    async () =>
      browser
        .execute(
          () =>
            !("__reloading" in window) && document.readyState === "complete",
        )
        .catch(() => false),
    { timeout: RELOAD_TIMEOUT, timeoutMsg: "app never finished reloading" },
  );
  await waitForHeading("Operating system");
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
      timeoutMsg: "no OS image became selectable — is the release catalog reachable?",
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

describe("flasher wizard", () => {
  let saved: Record<string, string | null>;

  before(async () => {
    const [main] = await browser.getWindowHandles();
    await browser.switchToWindow(main);
    saved = await readStorage();
    await reloadPristine();
  });

  after(async () => {
    await writeStorage(saved);
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

  it("prefills a unique hostname so Next is never dead on arrival", async () => {
    await reloadPristine();
    await onNetworkStep();
    await expect($("#hostname")).toHaveValue(/^aircast-[0-9a-f]{6}$/);
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

  it("rejects an invalid hostname with the rule it broke", async () => {
    await knownGoodNetworkState();
    await $("#hostname").setValue("Falcon_01");
    await expect($("#hostname-help")).toHaveText(
      expect.stringContaining("lowercase letters, numbers, and hyphens"),
    );
    await expect($("button=Next")).toBeDisabled();
  });

  it("refuses a WiFi password with no network name", async () => {
    await knownGoodNetworkState();
    await $("#ssid").clearValue();
    await $("#password").setValue("hunter2hunter2");
    await expect($("#ssid-error")).toHaveText(
      expect.stringContaining("Add the network name"),
    );
    await expect($("button=Next")).toBeDisabled();
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
    await $("button*=Device access").click();
    await $("button=Password").click();
    await expect($("#device-password")).toHaveValue("");
    await expect($("button=Next")).toBeEnabled();
    await $("button=SSH key").click();
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
});

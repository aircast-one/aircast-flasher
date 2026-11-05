import fs from "node:fs";
import net, { type AddressInfo } from "node:net";
import path from "node:path";

import { browser, $, $$, expect } from "@wdio/globals";

import { e2eConfigDir, settingsFile } from "./settings-file";

const HEADING_TIMEOUT = 30_000;
const CATALOG_TIMEOUT = 60_000;

async function heading(): Promise<string> {
  return $("h1")
    .getText()
    .catch(() => "");
}

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

const BACKUP_ID = "falcon-20261101120000";
const CARD_BACKUP_ID = "falcon-20261102120000";

function backupFile(id = BACKUP_ID): string {
  return path.join(e2eConfigDir(), "backups", `${id}.json`);
}

function seedBackup() {
  fs.mkdirSync(path.dirname(backupFile()), { recursive: true });
  fs.writeFileSync(
    backupFile(),
    JSON.stringify({
      format: 1,
      hostname: "falcon",
      created: "2026-11-01T12:00:00Z",
      files: [],
    }),
    { mode: 0o600 },
  );
}

const OWEL_BACKUP_ID = "owel-02-20261103120000";

function seedOtherDrone() {
  fs.mkdirSync(path.dirname(backupFile(OWEL_BACKUP_ID)), { recursive: true });
  fs.writeFileSync(
    backupFile(OWEL_BACKUP_ID),
    JSON.stringify({
      format: 1,
      hostname: "owel-02",
      created: "2026-11-03T12:00:00Z",
      files: [],
    }),
    { mode: 0o600 },
  );
}

const dronePicker = () => $('[aria-labelledby="restore-drone-label"]');

async function pickDrone(hostname: string) {
  await dronePicker().$(`label*=${hostname}`).click();
  await expect(dronePicker().$(`label*=${hostname}`).$('[role="radio"]')).toHaveAttribute(
    "aria-checked",
    "true",
  );
}

function seedCardBackup() {
  fs.mkdirSync(path.dirname(backupFile(CARD_BACKUP_ID)), { recursive: true });
  fs.writeFileSync(
    backupFile(CARD_BACKUP_ID),
    JSON.stringify({
      format: 1,
      hostname: "falcon",
      created: "2026-11-02T12:00:00Z",
      card: {
        fightMode: true,
        passwordLogin: true,
        encryption: {
          threshold: 1,
          tangUrls: [
            "http://127.0.0.1:1",
            "https://api.aircast.one/v1/tang/3f2a",
          ],
        },
        controlServer: "",
        tailnetNode: true,
      },
      files: [],
    }),
    { mode: 0o600 },
  );
}

function savedSettings(): Record<string, unknown> {
  return fs.existsSync(settingsFile())
    ? JSON.parse(fs.readFileSync(settingsFile(), "utf8"))
    : {};
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
  if ((await heading()) === "Network") return;
  const back = $('nav[aria-label="Setup steps"]').$("button*=Network");
  if (await back.isEnabled()) {
    await back.click();
  } else {
    await browser.waitUntil(
      async () => {
        await $("button=Next").click();
        return (await heading()) === "Network";
      },
      {
        timeout: CATALOG_TIMEOUT,
        interval: 1000,
        timeoutMsg:
          "no OS image became selectable — is the release catalog reachable?",
      },
    );
  }
  await waitForHeading("Network");
}

const problems = () => $('[data-testid="step-problems"]');

async function expectBlocked(button: string, mentions: string) {
  const before = await heading();
  await $(`button=${button}`).click();
  await expect(problems()).toBeDisplayed();
  await expect(problems()).toHaveText(expect.stringContaining(mentions));
  expect(await heading()).toBe(before);
}

async function useNewestRelease() {
  if ((await heading()) !== "Operating system") {
    await backToStep("Operating system");
  }
  await $('[aria-label="Aircast OS version"]').click();
  await $$('[role="option"]')[0].click();
}

async function backToStep(name: string) {
  await $('nav[aria-label="Setup steps"]').$(`button*=${name}`).click();
  await waitForHeading(name);
}

async function onAccessStep() {
  if ((await heading()) === "Access") return;
  await knownGoodNetworkState();
  await $("button=Next").click();
  await waitForHeading("Access");
}

async function knownGoodNetworkState() {
  await onNetworkStep();
  await $("#hostname").setValue("falcon-01");
  await $("#ssid").setValue("test-net");
  await $("#password").clearValue();
}

async function showsApp(): Promise<boolean> {
  return browser
    .execute(() => Boolean(document.getElementById("root")))
    .catch(() => false);
}

async function attachToAppWindow() {
  await browser
    .waitUntil(
      async () => {
        const handles = await browser.getWindowHandles();
        for (const handle of handles) {
          await browser.switchToWindow(handle);
          if (await showsApp()) return true;
        }
        return false;
      },
      { timeout: HEADING_TIMEOUT },
    )
    .catch(async () => {
      throw new Error(`no window held the app. ${await pageDiagnostics()}`);
    });
}

async function openRemoteAccess() {
  const toggle = $('[role="switch"][aria-label="Remote access"]');
  if ((await toggle.getAttribute("aria-checked")) !== "true") {
    await toggle.click();
  }
}

async function openAdvanced() {
  const advanced = $("#access-advanced");
  if ((await advanced.getAttribute("open")) === null) {
    await advanced.$("summary").click();
  }
}

describe("flasher wizard", () => {
  before(async () => {
    await attachToAppWindow();
  });

  it("asks before any diagnostics leave the machine, and only once", async () => {
    await waitForHeading("Diagnostics");
    const consent = $('[data-testid="consent-step"]');
    await expect(consent).toHaveText(
      expect.stringContaining("never includes your network name"),
    );
    await expect(
      $('nav[aria-label="Setup steps"]').$("button*=Network"),
    ).toBeDisabled();

    await $("button=No thanks").click();

    await waitForHeading("Operating system");
    await expect(consent).not.toBeDisplayed();
    await expect($("button=Diagnostics off")).toBeDisplayed();
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
      "Network",
      "Access",
      "Storage",
      "Write",
    ]);
  });

  it("asks which board the card is for, and fits the defaults to it", async () => {
    if ((await heading()) !== "Operating system") {
      await backToStep("Operating system");
    }
    const boards = $('[aria-label="Board"]');
    const pickMaker = async (name: string) => {
      await boards.$(`button*=${name}`).click();
      await expect(boards.$(`button*=${name}`)).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    };
    await expectBlocked("Next", "Choose the board this card is for");

    await pickMaker("Orange Pi");
    await expect(
      $('[aria-label="Model"] [data-board="orangepi5-plus"]'),
    ).toHaveText(expect.stringContaining("Orange Pi 5 Plus"));

    await pickMaker("Radxa");
    await expectBlocked("Next", "Choose the Radxa model");
    await expect($('[aria-label="Model"] [data-board="rock-5b"] img')).toBeDisplayed();
    await $('[aria-label="Model"] [data-board="rock-5b"]').click();
    await browser.waitUntil(
      () =>
        fs.existsSync(settingsFile()) &&
        JSON.parse(fs.readFileSync(settingsFile(), "utf8")).board === "rock-5b",
      {
        timeout: 10_000,
        timeoutMsg: "the picked board never reached settings.json",
      },
    );
    await expect($("button*=Aircast OS for Radxa ROCK 5B")).toHaveText(
      expect.stringContaining(".img.xz"),
    );
    await expect($("p*=support is in beta")).toBeDisplayed();

    await $("button=Next").click();
    await waitForHeading("Network");
    await expect($("p*=Radxa ROCK 5B has no built-in WiFi")).toBeDisplayed();

    await backToStep("Operating system");
    await pickMaker("Raspberry Pi");
    await expect($('[aria-label="Aircast OS version"]')).toBeDisplayed();
  });

  it("arrives with a hostname already filled in", async () => {
    await onNetworkStep();
    await expect($("#hostname")).not.toHaveValue("");
  });

  it("names a missing network on Next and clears it once named", async () => {
    await onNetworkStep();
    await $("#ssid").clearValue();
    await expectBlocked("Next", "WiFi network");
    await $("#ssid").setValue("test-net");
    await expect(problems()).not.toBeDisplayed();
  });

  it("says why it blocks when the hostname is cleared", async () => {
    await knownGoodNetworkState();
    await $("#hostname").clearValue();
    await expect($("#hostname-help")).toHaveText(
      "Enter a hostname to continue.",
    );
    await expectBlocked("Next", "Enter a hostname");
    await problems().$("button=Enter a hostname").click();
    await expect($("#hostname")).toBeFocused();
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
    await expectBlocked("Next", "lowercase letters, numbers and hyphens");
  });

  it("refuses to write a card with no network at all", async () => {
    await knownGoodNetworkState();
    await $("#ssid").clearValue();
    await expect($("#ssid-help")).toHaveText(
      expect.stringContaining("never comes online"),
    );
    await expectBlocked("Next", "WiFi network");
  });

  it("lets a wired device skip WiFi on purpose", async () => {
    await knownGoodNetworkState();
    await $("#ssid").clearValue();
    await $("button*=Ethernet or cellular only").click();
    await $("button=Next").click();
    await waitForHeading("Access");

    await backToStep("Network");
    await $("button*=Set up WiFi instead").click();
    await expect($("#ssid")).toBeDisplayed();
    await expectBlocked("Next", "WiFi network");
  });

  it("refuses a WiFi password the device could never join with", async () => {
    await knownGoodNetworkState();
    await $("#password").setValue("short");
    await expect($("#password-help")).toHaveText(
      expect.stringContaining("8–63 characters"),
    );
    await expectBlocked("Next", "8–63 characters");

    await $("#password").setValue("longenough");
    await expect(problems()).not.toBeDisplayed();
    await $("#password").clearValue();
  });

  it("lists every known network even when one is already picked", async () => {
    await knownGoodNetworkState();
    await $("#ssid").clearValue();
    await $('[aria-label="Show suggestions"]').click();
    const known = await $$('[role="option"]').map((o) => o.getText());
    await browser.keys("Escape");

    await $("#ssid").setValue(known[0]);
    await browser.keys("Escape");
    await $('[aria-label="Show suggestions"]').click();
    await expect($$('[role="option"]')).toBeElementsArrayOfSize(known.length);
    await browser.keys("Escape");
  });

  it("tests the WiFi from this computer and says when it is out of range", async () => {
    await knownGoodNetworkState();
    await $("#ssid").setValue("aircast-e2e-not-here");
    await $("#password").setValue("longenough");
    await $("button=Test from this computer").click();
    await expect($("#wifi-test-result")).toHaveText(
      expect.stringContaining(
        "aircast-e2e-not-here is not in range of this computer",
      ),
      { wait: 40_000 },
    );
    await $("#password").clearValue();
  });

  it("offers remote access before anyone asks for it", async () => {
    await onAccessStep();
    await expect(
      $('[role="switch"][aria-label="Remote access"]'),
    ).toHaveAttribute("aria-checked", "true");
    await expect($("#auth-key")).toBeDisplayed();
  });

  it("warns about a Headscale key with no control server without blocking", async () => {
    await onAccessStep();
    await openRemoteAccess();
    await $("button=Headscale").click();
    await $("#auth-key").setValue("hskey-auth-abc123");
    await expect($("#control-server-error")).toHaveText(
      expect.stringContaining("Enter your control server URL"),
    );
    await $("button=Next").click();
    await waitForHeading("Storage");

    await backToStep("Access");
    await $("button=Tailscale").click();
    await $("#auth-key").clearValue();
  });

  it("lists the Headscale users with the API key, and says when the server can't be reached", async () => {
    const closed = net.createServer();
    await new Promise<void>((resolve) => closed.listen(0, "127.0.0.1", resolve));
    const { port } = closed.address() as AddressInfo;
    await new Promise((resolve) => closed.close(resolve));

    await onAccessStep();
    await openRemoteAccess();
    await $("button=Headscale").click();
    await $("#control-server").setValue(`http://127.0.0.1:${port}`);
    await $("#auth-key").setValue("hskey-api-abc-def");
    await expect($("#auth-key-help")).toHaveText(
      expect.stringContaining("its own one-time key"),
    );
    await expect($("#headscale-user-error")).toHaveText(
      expect.stringContaining("Couldn't reach Headscale"),
    );
    await expectBlocked("Next", "Headscale user");

    await $("#auth-key").clearValue();
    await $("#control-server").clearValue();
    await $("button=Tailscale").click();
  });

  it("mints a key per card from a Tailscale API access token", async () => {
    await onAccessStep();
    await openRemoteAccess();
    await $("button=Tailscale").click();
    await $("#auth-key").setValue("tskey-api-abc-def");
    await expect($("#auth-key-help")).toHaveText(
      expect.stringContaining("its own one-time key"),
    );
    await expect($("#headscale-user")).not.toBeDisplayed();
    await expect(problems()).not.toBeDisplayed();
    await $("#auth-key").clearValue();
  });

  it("keeps the caret where it is when editing the control server", async () => {
    await onAccessStep();
    await openRemoteAccess();
    await $("button=Headscale").click();
    const field = await $("#control-server");
    await field.setValue("https://hs.example.com");
    await browser.execute((el: HTMLElement) => {
      const input = el as HTMLInputElement;
      input.focus();
      input.setSelectionRange(8, 8);
      document.execCommand("insertText", false, "a");
    }, field);
    await expect(field).toHaveValue("https://ahs.example.com");
    const caret = await browser.execute(
      (el: HTMLElement) => (el as HTMLInputElement).selectionStart,
      field,
    );
    expect(caret).toBe(9);
    await field.clearValue();
  });

  it("remembers every access choice, and switching one off erases nothing", async () => {
    await onAccessStep();
    await openRemoteAccess();
    await $("button=Headscale").click();
    await $("#control-server").setValue("https://hs.example.com");
    await $("#auth-key").setValue("hskey-auth-keep");
    await openAdvanced();
    await $("button=Password").click();
    await $("#device-password").setValue("drone-pass-1");
    await browser.waitUntil(
      () => {
        const saved = savedSettings();
        return (
          saved.remoteAccess !== false &&
          saved.selfHosted === true &&
          saved.controlServer === "https://hs.example.com" &&
          saved.authKey === "hskey-auth-keep" &&
          saved.sshMode === "password" &&
          saved.devicePassword === "drone-pass-1"
        );
      },
      { timeout: 10_000, timeoutMsg: `access choices never reached ${settingsFile()}` },
    );

    const remote = $('[role="switch"][aria-label="Remote access"]');
    await $("button=Tailscale").click();
    await remote.click();
    await browser.waitUntil(() => savedSettings().remoteAccess === false, {
      timeout: 10_000,
      timeoutMsg: "turning remote access off was not saved",
    });
    expect(savedSettings().controlServer).toBe("https://hs.example.com");
    expect(savedSettings().authKey).toBe("hskey-auth-keep");

    await remote.click();
    await $("button=Headscale").click();
    await expect($("#control-server")).toHaveValue("https://hs.example.com");
    await expect($("#auth-key")).toHaveValue("hskey-auth-keep");

    await $("#control-server").clearValue();
    await $("#auth-key").clearValue();
    await $("button=Tailscale").click();
    await $("#device-password").clearValue();
    await $("button=SSH key").click();
  });

  it("never prefills the stock device password", async () => {
    await onAccessStep();
    await openAdvanced();
    await $("button=Password").click();
    await expect($("#device-password")).toHaveValue("");
    await $("button=Next").click();
    await waitForHeading("Storage");

    await backToStep("Access");
    await openAdvanced();
    await $("button=SSH key").click();
  });

  it("shows the login the device will ship with, without being asked", async () => {
    await onAccessStep();
    await expect($("#access-advanced summary")).toHaveText(
      expect.stringContaining("SSH login: SSH key"),
    );
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

  it("fight mode changes the card, not what this computer remembers", async () => {
    await useNewestRelease();
    await onAccessStep();
    await browser.waitUntil(() => savedSettings().hostname === "falcon-01", {
      timeout: 10_000,
      timeoutMsg: "the drone name never reached the settings file",
    });

    const fight = $('[role="switch"][aria-label="Fight mode"]');
    await fight.click();

    await onNetworkStep();
    await $("#hostname").setValue("fight-01");
    await browser.waitUntil(() => savedSettings().hostname === "fight-01", {
      timeout: 10_000,
      timeoutMsg: `fight mode stopped settings reaching ${settingsFile()}`,
    });
    expect(savedSettings().ssid).toBe("test-net");

    await knownGoodNetworkState();
    await $("button=Next").click();
    await waitForHeading("Access");
    await fight.click();
  });

  it("fight mode takes a password, as long as it's long", async () => {
    await useNewestRelease();
    await onAccessStep();
    const fight = $('[role="switch"][aria-label="Fight mode"]');
    await fight.click();
    await $("button=Tailscale").click();
    await $("#auth-key").setValue("tskey-auth-abc123");

    await openAdvanced();
    await $("button=Password").click();
    await $("#device-password").setValue("drone-pass1");
    await expect($("#device-password-help")).not.toHaveText(
      expect.stringContaining("Use at least"),
    );
    await expect($("#device-password")).toHaveAttribute("aria-invalid", "false");
    await expectBlocked("Next", "at least 12 characters");
    await expect($("#device-password-help")).toHaveText(
      "Use at least 12 characters.",
    );

    await $("#device-password").setValue("drone-pass12");
    await expect(problems()).not.toBeDisplayed();

    await $("#device-password").clearValue();
    await $("#auth-key").clearValue();
    await $("button=SSH key").click();
    await fight.click();
  });

  it("won't let fight mode write a drone nothing can reach", async () => {
    await useNewestRelease();
    await onAccessStep();
    const fight = $('[role="switch"][aria-label="Fight mode"]');
    const remote = $('[role="switch"][aria-label="Remote access"]');
    await fight.click();

    await expect(remote).toHaveAttribute("aria-checked", "true");
    await expect(remote).toHaveAttribute("aria-disabled", "true");
    await expect($("#fight-mode-help")).toHaveText(
      expect.stringContaining("Finish Remote access"),
    );
    await expectBlocked("Next", "pre-auth key");

    await $("button=Tailscale").click();
    await $("#auth-key").setValue("tskey-auth-abc123");
    await expect($("#fight-mode-help")).not.toBeDisplayed();
    await expect(problems()).not.toBeDisplayed();

    await $("#auth-key").clearValue();
    await expect(problems()).toBeDisplayed();
    await fight.click();
    await expect(remote).not.toHaveAttribute("aria-disabled", "true");
    await expect(problems()).not.toBeDisplayed();
  });

  it("shows what will be written before the destructive step", async () => {
    await onAccessStep();
    await $("button=Next").click();

    await waitForHeading("Storage");
    await expect($("h2=What will be written")).toBeDisplayed();
    const rows = $("dl");
    await expect(rows).toHaveText(expect.stringContaining("falcon-01.local"));
    await expect(rows).toHaveText(expect.stringContaining("test-net"));
    await expect(rows).toHaveText(expect.stringContaining("Target"));
    await expect(rows).toHaveText(expect.stringContaining("aircastd"));
    await expect(rows).toHaveText(
      expect.stringContaining("Off · sign in to link it to your account"),
    );
    await expect(rows.$("button=Sign in with Google")).toBeDisplayed();
    await expectBlocked("Flash SD Card", "Insert an SD card");
  });

  it("offers writing straight to a Rockchip board over USB", async () => {
    await backToStep("Operating system");
    const boards = $('[aria-label="Board"]');
    await boards.$("button*=Radxa").click();
    await $('[aria-label="Model"] [data-board="rock-5b"]').click();
    await expect($("button*=Aircast OS for Radxa ROCK 5B")).toHaveText(
      expect.stringContaining(".img.xz"),
    );
    await $("button=Next").click();
    await waitForHeading("Network");
    await $("button=Next").click();
    await waitForHeading("Access");
    await $("button=Next").click();
    await waitForHeading("Storage");

    await expect($("h2*=Write straight to the board over USB")).toBeDisplayed();
    await expect($("p*=Waiting for the Radxa ROCK 5B in flashing mode")).toBeDisplayed();
    await expectBlocked(
      "Flash SD Card",
      "Insert an SD card or connect the board in flashing mode",
    );

    await backToStep("Operating system");
    await boards.$("button*=Raspberry Pi").click();
    await expect($('[aria-label="Aircast OS version"]')).toBeDisplayed();
  });

  it("restores a saved backup onto the card, as the same drone or a new one", async () => {
    seedBackup();
    seedOtherDrone();
    await knownGoodNetworkState();
    await $("button=Next").click();
    await waitForHeading("Access");
    await backToStep("Network");

    await $('[role="switch"][aria-label="Restore settings"]').click();
    await expect(dronePicker().$$('[role="radio"]')).toBeElementsArrayOfSize(2);
    await expect(
      dronePicker().$('[role="radio"][aria-checked="true"]'),
    ).not.toBeExisting();
    await expect($("button*=Same drone")).not.toBeExisting();
    await expectBlocked("Next", "Pick the drone whose backup goes on this card");
    await problems().$("button*=Pick the drone").click();
    await browser.waitUntil(
      () =>
        browser.execute(
          () =>
            document.activeElement?.getAttribute("role") === "radio" &&
            document.activeElement.closest(
              '[aria-labelledby="restore-drone-label"]',
            ) !== null,
        ),
      { timeout: 5_000, timeoutMsg: "the error summary didn't move focus to the drone picker" },
    );

    await pickDrone("falcon");
    await expect(problems()).not.toHaveText(
      expect.stringContaining("Pick the drone"),
    );
    await expectBlocked("Next", "same drone or a new one");

    await $("button*=New drone").click();
    await expect($("#hostname")).toHaveValue("falcon-01");
    await $("button*=Same drone").click();
    await expect($("#hostname")).toHaveValue("falcon");

    await $("button=Next").click();
    await waitForHeading("Access");
    await $("button=Next").click();
    await waitForHeading("Storage");
    await expect($("dl")).toHaveText(expect.stringContaining("same drone"));
    expect(JSON.stringify(savedSettings())).not.toContain(BACKUP_ID);

    await backToStep("Network");
    await $('[role="switch"][aria-label="Restore settings"]').click();
    await $("#hostname").setValue("falcon-01");
    fs.rmSync(backupFile(OWEL_BACKUP_ID));
  });

  it("puts the restore first on the Network step, and says when a backup records nothing about the card", async () => {
    seedBackup();
    await knownGoodNetworkState();
    const restoreFirst = await browser.execute(() => {
      const restore = document.querySelector('[aria-label="Restore settings"]');
      const ssid = document.getElementById("ssid");
      return Boolean(
        restore &&
          ssid &&
          restore.compareDocumentPosition(ssid) &
            Node.DOCUMENT_POSITION_FOLLOWING,
      );
    });
    expect(restoreFirst).toBe(true);

    await $('[role="switch"][aria-label="Restore settings"]').click();
    await expect($("#restore-no-card")).not.toBeExisting();
    await pickDrone("falcon");
    await expect($("#restore-no-card")).toHaveText(
      expect.stringContaining("doesn't record fight mode, encryption or remote access"),
    );
    await $('[role="switch"][aria-label="Restore settings"]').click();
  });

  it("brings a backup's fight mode, encryption and tailnet node back onto the card", async () => {
    seedCardBackup();
    seedBackup();
    await useNewestRelease();
    await knownGoodNetworkState();
    await $("button=Next").click();
    await waitForHeading("Access");
    await backToStep("Network");

    await $('[role="switch"][aria-label="Restore settings"]').click();
    await pickDrone("falcon");
    await expect($("#restore-chosen")).toHaveText(
      expect.stringContaining("newest backup"),
    );
    await expect($("#restore-no-card")).not.toBeExisting();
    const versions = $("#restore-versions");
    await expect(versions).not.toHaveAttribute("open");
    await versions.$("summary").click();
    await expect(versions.$$('[role="radio"]')).toBeElementsArrayOfSize(2);
    await versions.$$("label")[1].click();
    await expect($("#restore-chosen")).toHaveText(
      expect.stringContaining("an earlier backup"),
    );
    await expect($("#restore-no-card")).toBeDisplayed();
    await versions.$$("label")[0].click();
    await expect($("#restore-no-card")).not.toBeExisting();

    await $("button*=Same drone").click();
    await expect($("#hostname")).toHaveValue("falcon");
    await $("button=Next").click();
    await waitForHeading("Access");

    await expect(
      $('[role="switch"][aria-label="Fight mode"]'),
    ).toHaveAttribute("aria-checked", "true");
    await expect($("h2*=Fight mode")).toHaveText(
      expect.stringContaining("from backup"),
    );
    await expect($("#remote-rejoins")).toHaveText(
      expect.stringContaining("rejoins your tailnet as the same node"),
    );
    await expect(
      $('[role="switch"][aria-label="Disk encryption"]'),
    ).toHaveAttribute("aria-checked", "true");
    await expect($("#tang-server-0")).toHaveValue("http://127.0.0.1:1");
    await expect($("#tang-server-1")).not.toBeExisting();
    await expect($("#access-advanced summary")).toHaveText(
      expect.stringContaining("SSH login: Password"),
    );
    await expect($("#device-password-hint")).toHaveText(
      expect.stringContaining("Backups don't carry the drone's password"),
    );
    await expect($("#device-password-help")).not.toHaveText(
      expect.stringContaining("Use at least"),
    );
    await expect($("label=New password for pi")).toBeDisplayed();
    await expectBlocked("Next", "at least 12 characters");
    await expect($("#device-password-help")).toHaveText(
      "Use at least 12 characters.",
    );
    await browser.waitUntil(
      () => savedSettings().tangCloud === true && savedSettings().sshMode === "password",
      { timeout: 10_000, timeoutMsg: "the backup's Cloud unlock and password login were not preset" },
    );

    await $('[role="switch"][aria-label="Disk encryption"]').click();
    await openAdvanced();
    await $("button=Off").click();
    await $("button=Next").click();
    await waitForHeading("Storage");
    const rows = $("dl");
    await expect(rows).toHaveText(
      expect.stringContaining("Rejoins your tailnet as falcon · no key needed"),
    );
    await expect(rows).toHaveText(expect.stringContaining("from backup"));
    await expect(rows).not.toHaveText(
      expect.stringContaining("needs remote access"),
    );

    await backToStep("Access");
    await $('[role="switch"][aria-label="Fight mode"]').click();
    await openAdvanced();
    await $("button=SSH key").click();
    await backToStep("Network");
    await $('[role="switch"][aria-label="Restore settings"]').click();
    await $("#hostname").setValue("falcon-01");
    fs.rmSync(backupFile(CARD_BACKUP_ID));
  });

  it("restores a saved backup onto a card straight from Devices", async () => {
    seedBackup();
    await $('nav[aria-label="Fleet"]').$("button*=Devices").click();
    await waitForHeading("Devices");
    await $(`[data-backup="${BACKUP_ID}"]`).$("button*=Restore onto a card").click();

    await waitForHeading("Network");
    await expect(
      $('[role="switch"][aria-label="Restore settings"]'),
    ).toHaveAttribute("aria-checked", "true");
    await expect(
      dronePicker().$("label*=falcon").$('[role="radio"]'),
    ).toHaveAttribute("aria-checked", "true");
    await expect($("#restore-chosen")).toHaveText(
      expect.stringContaining("falcon's newest backup"),
    );
    await expect($("button*=Same drone")).toHaveAttribute("aria-pressed", "false");
    await expect($("button*=New drone")).toHaveAttribute("aria-pressed", "false");
    await expectBlocked("Next", "same drone or a new one");
    await $('[role="switch"][aria-label="Restore settings"]').click();
  });

  it("offers background backups once there is a backup, and remembers Not now", async () => {
    seedBackup();
    await $('nav[aria-label="Fleet"]').$("button*=Devices").click();
    await waitForHeading("Devices");

    const prompt = $("#background-prompt");
    await expect(prompt).toHaveText(
      expect.stringContaining("Keep backing up your drones in the background?"),
    );
    const toggle = $(
      '[role="switch"][aria-label="Keep backing up drones in the background"]',
    );
    await expect(toggle).toHaveAttribute("aria-checked", "false");

    await prompt.$("button=Not now").click();
    await expect(prompt).not.toBeExisting();
    await browser.waitUntil(() => savedSettings().backgroundBackups === false, {
      timeout: 10_000,
      timeoutMsg: "Not now was not remembered — the prompt would come back",
    });
    await expect(toggle).toHaveAttribute("aria-checked", "false");
    await $('nav[aria-label="Fleet"]').$("button*=Devices").click();
  });

  it("lists saved backups under Devices, and deletes one only when confirmed", async () => {
    seedBackup();
    await $('nav[aria-label="Fleet"]').$("button*=Devices").click();
    await waitForHeading("Devices");

    const row = $(`[data-backup="${BACKUP_ID}"]`);
    await expect(row).toHaveText(expect.stringContaining("falcon"));
    await row.$('button[aria-label="Delete the falcon backup"]').click();
    expect(fs.existsSync(backupFile())).toBe(true);
    await row.$("button=Delete backup").click();
    await expect(row).not.toBeExisting();
    expect(fs.existsSync(backupFile())).toBe(false);

    await $('nav[aria-label="Fleet"]').$("button*=Devices").click();
  });

  it("sends you back to the step that owns a setting you want to change", async () => {
    await onAccessStep();
    await $("button=Next").click();
    await waitForHeading("Storage");

    await $('button[aria-label="Change Image"]').click();
    await waitForHeading("Operating system");
  });
});

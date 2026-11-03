import assert from "node:assert/strict";
import { test } from "node:test";

import { devicePage, setupUrl, tailnetDevice } from "./device-address.ts";

const peer = { hostName: "falcon", dnsName: "falcon.tail1234.ts.net.", os: "linux", online: true, ip: "100.64.0.9", sameTailnet: true };

test("continuing setup opens the device's own setup wizard", () => {
  assert.equal(setupUrl("http://falcon.local"), "http://falcon.local/fly/#device=%2Fsetup");
  assert.equal(setupUrl("http://10.42.0.1/"), "http://10.42.0.1/fly/#device=%2Fsetup");
});

test("a device page opens on the drone the screen found", () => {
  assert.equal(devicePage("http://falcon.tail1234.ts.net", "/streaming"), "http://falcon.tail1234.ts.net/fly/#device=%2Fstreaming");
  assert.equal(devicePage("http://falcon.local/", "/settings/device"), "http://falcon.local/fly/#device=%2Fsettings%2Fdevice");
});

test("a drone found on the tailnet is reached by its tailnet name, not an IP", () => {
  const found = tailnetDevice(peer, "falcon");
  assert.equal(found.url, "http://falcon.tail1234.ts.net");
  assert.equal(found.address, "falcon.tail1234.ts.net");
  assert.equal(found.via, "tailnet");
});

test("a tailnet that renamed the drone says so", () => {
  assert.match(tailnetDevice({ ...peer, hostName: "falcon-1" }, "falcon").note, /as falcon-1 — falcon was taken/);
});

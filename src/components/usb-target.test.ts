import assert from "node:assert/strict";
import { test } from "node:test";

import { fitsBoard, isUsbTarget, usbFamily } from "./usb-target.ts";
import type { BlockDevice, Board } from "../types.ts";

const board = (detail: string): Board => ({
  id: "x",
  name: "X",
  maker: "Radxa",
  detail,
  builtinWifi: false,
  beta: true,
  tested: false,
});

const device = (path: string): BlockDevice => ({
  path,
  name: "d",
  size: 0,
  size_human: "",
  removable: true,
  mounted: false,
  mount_points: [],
});

test("a board's chip names its USB family", () => {
  assert.equal(usbFamily(board("Rockchip RK3566")), "rk356x");
  assert.equal(usbFamily(board("Rockchip RK3568")), "rk356x");
  assert.equal(usbFamily(board("Rockchip RK3588S")), "rk3588");
  assert.equal(usbFamily(board("Rockchip RK3582")), "rk3588");
  assert.equal(usbFamily(board("Pi 3, 4, 5 and Zero 2 W")), null);
  assert.equal(usbFamily(null), null);
});

test("a USB board is a target only for an image of its own chip family", () => {
  const zero3 = board("Rockchip RK3566");
  assert.ok(isUsbTarget("usb:rk356x") && !isUsbTarget("/dev/disk4"));
  assert.ok(fitsBoard(device("usb:rk356x"), zero3));
  assert.ok(!fitsBoard(device("usb:rk3588"), zero3));
  assert.ok(!fitsBoard(device("usb:rk356x"), board("Pi 3, 4, 5 and Zero 2 W")));
  assert.ok(fitsBoard(device("/dev/disk4"), zero3));
  assert.ok(fitsBoard(device("usb:rk3588"), null));
});

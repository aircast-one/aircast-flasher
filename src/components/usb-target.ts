import type { BlockDevice, Board } from "@/types";

export const isUsbTarget = (path: string): boolean => path.startsWith("usb:");

export const usbFamily = (board: Board | null): string | null =>
  board === null
    ? null
    : /RK356/.test(board.detail)
      ? "rk356x"
      : /RK358/.test(board.detail)
        ? "rk3588"
        : null;

export const fitsBoard = (device: BlockDevice, board: Board | null): boolean =>
  !isUsbTarget(device.path) ||
  board === null ||
  device.path === `usb:${usbFamily(board)}`;

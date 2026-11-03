import type { RestoredCard } from "@/components/device-finder";
import type { Board } from "@/types";

export function troubleshootingTips(p: {
  board: Board | null;
  usb: boolean;
  noWifi: boolean;
  fightMode: boolean;
  restore: RestoredCard | null;
}): string[] {
  return [
    ...(p.fightMode
      ? [
          p.restore?.rejoins
            ? `Check it has power and is in range of its WiFi — it rejoins as ${p.restore.host} with the identity from the backup.`
            : "The pre-auth key was probably expired or already used on another card. Make a fresh key and write the card again.",
        ]
      : []),
    ...(p.board !== null && !p.board.builtinWifi && !p.noWifi
      ? [
          `${p.board.name} has no built-in WiFi. Plug in a USB WiFi adapter or an Ethernet cable, then power it on again.`,
        ]
      : []),
    ...(p.board !== null && !p.usb
      ? [
          `This card holds the ${p.board.name} image. It won't start on a different board.`,
        ]
      : []),
  ];
}

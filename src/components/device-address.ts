import type { TailnetPeer } from "@/types";

export type FoundDevice = {
  url: string;
  address: string;
  via: "lan" | "hotspot" | "tailnet";
  note: string;
};

export const setupUrl = (deviceUrl: string): string =>
  `${deviceUrl.replace(/\/+$/, "")}/fly/#device=${encodeURIComponent("/setup")}`;

export const tailnetDevice = (peer: TailnetPeer, hostname: string): FoundDevice => {
  const address = peer.dnsName.replace(/\.$/, "");
  const wanted = hostname.trim();
  return {
    url: `http://${address}`,
    address,
    via: "tailnet",
    note:
      peer.hostName.toLowerCase() === wanted.toLowerCase()
        ? "On your private network, so you can reach it from anywhere."
        : `On your private network as ${peer.hostName} — ${wanted} was taken.`,
  };
};

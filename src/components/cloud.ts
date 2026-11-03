import type {
  CloudCard,
  CloudKeyset,
  CloudSignInStatus,
  EncryptionConfig,
} from "@/types";

export type CardState = "lost" | "unlocked" | "never-unlocked";

export const cardState = (card: CloudCard): CardState =>
  card.lostAt !== null
    ? "lost"
    : card.lastUnlockAt !== null
      ? "unlocked"
      : "never-unlocked";

export const waitForSignIn = (
  token: string,
  poll: (token: string) => Promise<CloudSignInStatus>,
  pause: () => Promise<void>,
): Promise<string> =>
  poll(token).then((status) =>
    status.status === "signedIn"
      ? status.email
      : pause().then(() => waitForSignIn(token, poll, pause)),
  );

export const cardLabel = (hostname: string) =>
  hostname.trim() === "" ? "unnamed card" : hostname.trim();

export const withCloudServer = (
  encryption: EncryptionConfig,
  keyset: CloudKeyset,
): EncryptionConfig => ({
  ...encryption,
  tangServers: [
    ...encryption.tangServers,
    { url: keyset.url, adv: JSON.stringify(keyset.advertisement) },
  ],
});

export const retireCandidates = (
  cards: CloudCard[],
  hostnames: string[],
  keep: string,
): CloudCard[] =>
  cards.filter(
    (card) =>
      card.id !== keep &&
      card.lostAt === null &&
      hostnames.some((host) => cardLabel(host) === card.label),
  );

export interface CloudLink {
  account: string | null;
  on: boolean;
  sameDrone: boolean;
  usb: boolean;
}

export type CloudOutcome = { account: string } | { error: string } | null;

export const linkedAccount = (link: CloudLink): string | null =>
  link.on && !link.sameDrone && !link.usb ? link.account : null;

export const cloudLinkSummary = (link: CloudLink): string =>
  link.account === null
    ? "Off · sign in to link it to your account"
    : link.usb
      ? "Off · connect it from its device page"
      : link.sameDrone
        ? "Keeps the link from the backup"
        : link.on
          ? `Links to ${link.account} on first start`
          : "Off";

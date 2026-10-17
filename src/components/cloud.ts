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

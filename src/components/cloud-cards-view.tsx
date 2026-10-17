import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Loader2, LogOut, RefreshCw } from "lucide-react";

import {
  cloudAccount,
  cloudListCards,
  cloudMarkLost,
  cloudSignInPoll,
  cloudSignInStart,
  cloudSignOut,
} from "@/api";
import { Button } from "@/components/ui/button";
import { cardState, waitForSignIn } from "@/components/cloud";
import type { CloudCard } from "@/types";

const SIGN_IN_POLL_MS = 2000;
const ACCOUNT_KEY = ["cloud-account"];
const CARDS_KEY = ["cloud-cards"];

const pause = () =>
  new Promise<void>((resolve) => setTimeout(resolve, SIGN_IN_POLL_MS));

const errorText = (e: unknown) =>
  typeof e === "string"
    ? e
    : e instanceof Error
      ? e.message
      : "Something went wrong.";

const when = (iso: string) => new Date(iso).toLocaleString();

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-xl border border-border px-4 py-6 text-center text-sm text-muted-foreground">
      {children}
    </div>
  );
}

function CardRow({ card }: { card: CloudCard }) {
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const markLost = useMutation({
    mutationFn: () => cloudMarkLost(card.id),
    onSettled: () => {
      setConfirming(false);
      void queryClient.invalidateQueries({ queryKey: CARDS_KEY });
    },
  });
  const state = cardState(card);

  return (
    <div className="flex flex-col gap-2 rounded-xl border border-border px-4 py-3">
      <div className="flex items-center gap-3">
        <span
          className={
            state === "lost"
              ? "size-2 shrink-0 rounded-full bg-destructive"
              : "size-2 shrink-0 rounded-full bg-green-500 dark:bg-green-400"
          }
        />
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-sm font-medium">{card.label}</span>
          <span className="truncate text-xs text-muted-foreground">
            {state === "lost" && card.lostAt !== null
              ? `Marked lost ${when(card.lostAt)} — it can no longer unlock`
              : state === "unlocked" && card.lastUnlockAt !== null
                ? `Last unlocked ${when(card.lastUnlockAt)}${card.lastUnlockIp ? ` from ${card.lastUnlockIp}` : ""}`
                : `Flashed ${when(card.createdAt)} — not unlocked yet`}
          </span>
        </div>
        {state === "lost" ? null : confirming ? (
          <div className="flex shrink-0 items-center gap-2">
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => setConfirming(false)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              size="sm"
              variant="destructive"
              disabled={markLost.isPending}
              onClick={() => markLost.mutate()}
            >
              {markLost.isPending ? <Loader2 className="size-4 animate-spin" /> : null}
              Mark lost — can't be undone
            </Button>
          </div>
        ) : (
          <Button
            type="button"
            size="sm"
            variant="secondary"
            className="shrink-0"
            onClick={() => setConfirming(true)}
          >
            Mark lost
          </Button>
        )}
      </div>
      {markLost.isError ? (
        <p className="text-sm text-destructive" role="alert">
          {errorText(markLost.error)}
        </p>
      ) : null}
    </div>
  );
}

export const useCloudAccount = () =>
  useQuery({ queryKey: ACCOUNT_KEY, queryFn: cloudAccount });

export function CloudSignInButton() {
  const queryClient = useQueryClient();
  const signIn = useMutation({
    mutationFn: async () => {
      const { token, authUrl } = await cloudSignInStart();
      await openUrl(authUrl);
      return waitForSignIn(token, cloudSignInPoll, pause);
    },
    onSuccess: (email) => queryClient.setQueryData(ACCOUNT_KEY, email),
  });

  return (
    <div className="flex flex-col items-start gap-2">
      <Button
        type="button"
        size="sm"
        disabled={signIn.isPending}
        onClick={() => signIn.mutate()}
      >
        {signIn.isPending ? <Loader2 className="size-4 animate-spin" /> : null}
        {signIn.isPending ? "Waiting for Google sign-in…" : "Sign in with Google"}
      </Button>
      {signIn.isError ? (
        <p className="text-sm text-destructive" role="alert">
          {errorText(signIn.error)}
        </p>
      ) : null}
    </div>
  );
}

function SignIn() {
  return (
    <Empty>
      <p>
        Sign in to Aircast Cloud to see the cards bound to its key server and
        mark a lost one so it can never unlock again.
      </p>
      <div className="mt-4 flex justify-center">
        <CloudSignInButton />
      </div>
    </Empty>
  );
}

function Cards() {
  const cards = useQuery({ queryKey: CARDS_KEY, queryFn: cloudListCards });

  if (cards.isPending) return <Empty>Loading cards…</Empty>;
  if (cards.isError) return <Empty>{errorText(cards.error)}</Empty>;
  if (cards.data.length === 0) {
    return (
      <Empty>
        No cards use the Aircast Cloud key server yet. Turn it on under Disk
        encryption when you flash one.
      </Empty>
    );
  }
  return (
    <>
      {cards.data.map((card) => (
        <CardRow key={card.id} card={card} />
      ))}
    </>
  );
}

export function CloudCardsView() {
  const queryClient = useQueryClient();
  const account = useCloudAccount();
  const signOut = useMutation({
    mutationFn: cloudSignOut,
    onSuccess: () => {
      queryClient.setQueryData(ACCOUNT_KEY, null);
      queryClient.removeQueries({ queryKey: CARDS_KEY });
    },
  });
  const email = account.data;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-start justify-between gap-4 px-8 pt-7 pb-4">
        <div className="flex flex-col gap-1">
          <h1 className="text-2xl font-bold tracking-tight">Cloud cards</h1>
          <p className="text-sm text-muted-foreground">
            Cards that unlock through the Aircast Cloud key server. A lost card
            stays readable by whoever holds it until you mark it lost here.
          </p>
        </div>
        {typeof email === "string" ? (
          <div className="flex shrink-0 items-center gap-2">
            <Button
              type="button"
              variant="secondary"
              onClick={() =>
                void queryClient.invalidateQueries({ queryKey: CARDS_KEY })
              }
            >
              <RefreshCw />
              Refresh
            </Button>
            <Button
              type="button"
              variant="ghost"
              title={`Signed in as ${email}`}
              onClick={() => signOut.mutate()}
            >
              <LogOut />
              Sign out
            </Button>
          </div>
        ) : null}
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-8 pb-8">
        {account.isPending ? (
          <Empty>Checking your Aircast Cloud sign-in…</Empty>
        ) : typeof email === "string" ? (
          <Cards />
        ) : (
          <SignIn />
        )}
      </div>
    </div>
  );
}

import { useEffect, useState } from "react";
import { Check, Loader2 } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import type { CloudOutcome } from "@/components/cloud";
import { devicePage, type FoundDevice } from "@/components/device-address";
import {
  ContinueSetup,
  DeviceOnline,
  DeviceWaiting,
  missingAfterMinutes,
  type RestoredCard,
  useFoundDevice,
} from "@/components/device-finder";
import { cn } from "@/lib/utils";

type StepState = "done" | "current" | "waiting" | "todo";

function StepMark({ state, number }: { state: StepState; number: number }) {
  return (
    <span
      className={cn(
        "flex size-6 shrink-0 items-center justify-center rounded-full text-xs font-medium",
        state === "done" && "bg-green-500/15 text-green-600 dark:text-green-400",
        state === "current" && "bg-primary text-primary-foreground",
        state === "waiting" && "bg-primary/10 text-primary",
        state === "todo" && "border border-border text-muted-foreground",
      )}
    >
      {state === "done" ? (
        <Check className="size-3.5" aria-label="done" />
      ) : state === "waiting" ? (
        <Loader2 className="size-3.5 animate-spin" aria-label="waiting" />
      ) : (
        number
      )}
    </span>
  );
}

function Step({
  number,
  state,
  title,
  last = false,
  children,
}: {
  number: number;
  state: StepState;
  title: string;
  last?: boolean;
  children?: React.ReactNode;
}) {
  return (
    <li
      className="flex gap-3"
      aria-current={state === "current" ? "step" : undefined}
    >
      <div className="flex flex-col items-center">
        <StepMark state={state} number={number} />
        {last ? null : <span className="mt-1 w-px flex-1 bg-border" />}
      </div>
      <div className={cn("flex min-w-0 flex-1 flex-col gap-2", !last && "pb-5")}>
        <span
          className={cn(
            "pt-0.5 text-sm font-medium",
            state === "todo" && "text-muted-foreground",
          )}
        >
          {title}
        </span>
        {children}
      </div>
    </li>
  );
}

function DevicePageLink({
  found,
  page,
  children,
}: {
  found: FoundDevice;
  page: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      className="underline underline-offset-2 hover:text-foreground"
      onClick={() => void openUrl(devicePage(found.url, page))}
    >
      {children}
    </button>
  );
}

function CloudStep({
  cloud,
  found,
}: {
  cloud: CloudOutcome;
  found: FoundDevice | null;
}) {
  if (cloud !== null && "account" in cloud)
    return (
      <Step number={4} state="done" title={`Linked to ${cloud.account}`}>
        <span className="text-xs text-muted-foreground">
          It signs in to Aircast Cloud on its first start.
        </span>
      </Step>
    );
  return (
    <Step
      number={4}
      state={found !== null ? "current" : "todo"}
      title="Connect it to your Aircast account"
    >
      <span className="text-xs text-muted-foreground">
        {cloud === null ? null : `Linking it while writing failed: ${cloud.error}. `}
        Do it from{" "}
        {found !== null ? (
          <DevicePageLink found={found} page="/settings/device">
            the device page
          </DevicePageLink>
        ) : (
          "the device page, once it's online"
        )}
        .
      </span>
    </Step>
  );
}

function useAfter(ms: number): boolean {
  const [passed, setPassed] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => setPassed(true), ms);
    return () => clearTimeout(timer);
  }, [ms]);
  return passed;
}

export function SetupSteps({
  hostname,
  usb,
  remoteEnrolled,
  fightMode,
  restore,
  cloud,
  notShowingUp,
}: {
  hostname: string;
  usb: boolean;
  remoteEnrolled: boolean;
  fightMode: boolean;
  restore: RestoredCard | null;
  cloud: CloudOutcome;
  notShowingUp: React.ReactNode;
}) {
  const { found, rounds } = useFoundDevice(hostname, fightMode, restore);
  const late = useAfter(missingAfterMinutes(restore) * 60 * 1000);
  const name = hostname.trim();

  return (
    <ol className="flex w-full flex-col text-left">
      <Step
        number={1}
        state="done"
        title={usb ? "Aircast OS installed" : "Card written and checked"}
      />
      <Step
        number={2}
        state={found !== null || usb ? "done" : "current"}
        title={
          usb
            ? "Board restarting into Aircast OS"
            : found !== null
              ? "Card in the drone and powered on"
              : "Put the card in the drone and power it on"
        }
      />
      <Step
        number={3}
        state={found !== null ? "done" : "waiting"}
        title={found !== null ? `${name} is online` : `Looking for ${name}…`}
      >
        {found !== null ? (
          <DeviceOnline found={found} />
        ) : (
          <>
            <DeviceWaiting
              hostname={hostname}
              remoteEnrolled={remoteEnrolled}
              fightMode={fightMode}
              restore={restore}
              rounds={rounds}
            />
            {late ? notShowingUp : null}
          </>
        )}
      </Step>
      <CloudStep cloud={cloud} found={found} />
      <Step
        number={5}
        state={found !== null ? "current" : "todo"}
        title={`Finish setting up ${name}`}
        last
      >
        <span className="text-xs text-muted-foreground">
          Add a camera on{" "}
          {found !== null ? (
            <DevicePageLink found={found} page="/streaming">
              the device page
            </DevicePageLink>
          ) : (
            "the device page"
          )}
          , then the flight controller, and you're flying.
        </span>
        {found !== null ? <ContinueSetup found={found} hostname={hostname} /> : null}
      </Step>
    </ol>
  );
}

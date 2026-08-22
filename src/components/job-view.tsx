import { useEffect, useState } from "react";
import {
  Check,
  CheckCircle2,
  Copy,
  HardDriveDownload,
  Loader2,
  OctagonX,
  ShieldCheck,
  XCircle,
  type LucideIcon,
} from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { formatBytes, formatSpeed } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { FlashProgressState } from "@/components/wizard-types";

function IndeterminateBar() {
  return (
    <div
      className="h-2 w-full overflow-hidden rounded-full bg-muted"
      role="progressbar"
      aria-label="Working"
    >
      <div className="bar-indeterminate h-full w-[35%] rounded-full bg-primary" />
    </div>
  );
}

function Pane({
  heading,
  children,
}: {
  heading: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="px-8 pt-7 pb-4">
        <h1 className="text-2xl font-bold tracking-tight">{heading}</h1>
      </div>
      <div className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto px-8 pb-8">
        <div className="flex w-full max-w-md flex-col items-center gap-6 text-center duration-300 animate-in fade-in-0 zoom-in-95">
          {children}
        </div>
      </div>
    </div>
  );
}

type IconTone = "primary" | "success" | "destructive";

const toneClasses: Record<IconTone, { ring: string; icon: string }> = {
  primary: {
    ring: "bg-primary/10 text-primary ring-primary/20",
    icon: "text-primary",
  },
  success: {
    ring: "bg-green-500/10 text-green-500 ring-green-500/20 dark:text-green-400",
    icon: "text-green-500 dark:text-green-400",
  },
  destructive: {
    ring: "bg-destructive/10 text-destructive ring-destructive/20",
    icon: "text-destructive",
  },
};

function IconBadge({
  icon: Icon,
  tone,
  anim = "none",
}: {
  icon: LucideIcon;
  tone: IconTone;
  anim?: "spin" | "pulse" | "none";
}) {
  const t = toneClasses[tone];
  return (
    <div
      className={cn(
        "flex size-16 items-center justify-center rounded-full ring-1",
        t.ring,
        anim === "pulse" && "animate-pulse",
      )}
    >
      <Icon
        className={cn("size-8", t.icon, anim === "spin" && "animate-spin")}
      />
    </div>
  );
}

const COPIED_FEEDBACK_MS = 2000;

function CopyableHostname({ hostname }: { hostname: string }) {
  const [copied, setCopied] = useState(false);
  const address = `${hostname}.local`;

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), COPIED_FEEDBACK_MS);
    return () => clearTimeout(timer);
  }, [copied]);

  return (
    <button
      type="button"
      className="flex items-center gap-2 self-start font-mono text-base break-all hover:text-primary"
      onClick={() => {
        navigator.clipboard
          ?.writeText(address)
          .then(() => setCopied(true))
          .catch(() => setCopied(false));
      }}
      aria-label={`Copy ${address}`}
    >
      {address}
      {copied ? (
        <Check className="size-4 shrink-0 text-green-500 dark:text-green-400" />
      ) : (
        <Copy className="size-4 shrink-0 text-muted-foreground" />
      )}
    </button>
  );
}

function ProgressPanel({
  icon,
  title,
  subtitle,
  stepLabel,
  percent,
  detail,
  determinate,
  spinner = false,
  onCancel,
  showCancel,
}: {
  icon: LucideIcon;
  title: string;
  subtitle: string;
  stepLabel: string | null;
  percent: number;
  detail: string | null;
  determinate: boolean;
  spinner?: boolean;
  onCancel: () => void;
  showCancel: boolean;
}) {
  const anim = spinner ? "spin" : determinate ? "none" : "pulse";
  return (
    <>
      <IconBadge icon={icon} tone="primary" anim={anim} />
      <div className="flex flex-col gap-1.5">
        {stepLabel ? (
          <span className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {stepLabel}
          </span>
        ) : null}
        <h2 className="text-lg font-semibold">{title}</h2>
        <p className="text-sm text-muted-foreground">{subtitle}</p>
      </div>

      <div className="flex w-full flex-col gap-2">
        {determinate ? (
          <Progress value={percent} className="w-full" />
        ) : (
          <IndeterminateBar />
        )}
        <div className="flex items-center justify-between text-xs text-muted-foreground tabular-nums">
          <span>{determinate ? `${percent}%` : "Working…"}</span>
          {detail ? <span>{detail}</span> : null}
        </div>
      </div>

      {showCancel ? (
        <Button type="button" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      ) : null}
    </>
  );
}

export function JobView({
  hostname,
  remoteEnrolled,
  success,
  cancelled,
  error,
  progress,
  onCancel,
  onRetry,
  onStartOver,
  onReset,
  onRevealLog,
}: {
  hostname: string;
  remoteEnrolled: boolean;
  success: boolean;
  cancelled: boolean;
  error: string | null;
  progress: FlashProgressState;
  onCancel: () => void;
  onRetry: (() => void) | null;
  onStartOver: () => void;
  onReset: () => void;
  onRevealLog: () => void;
}) {
  if (success) {
    return (
      <Pane heading="Write">
        <IconBadge icon={CheckCircle2} tone="success" />
        <div className="flex flex-col gap-1.5">
          <h2 className="text-xl font-semibold">Ready to go</h2>
          <p className="text-sm text-balance text-muted-foreground">
            Your SD card has been flashed and verified. Remove it and boot your
            device.
          </p>
        </div>
        <div className="flex w-full flex-col gap-1 rounded-xl border border-border px-4 py-3">
          <span className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            Find it at
          </span>
          <CopyableHostname hostname={hostname} />
          {remoteEnrolled ? (
            <span className="text-xs text-muted-foreground">
              It also joins your private network on first boot.
            </span>
          ) : null}
        </div>
        <div className="w-full text-left text-sm text-muted-foreground">
          <h3 className="mb-1 font-medium text-foreground">What happens next</h3>
          <ol className="list-decimal space-y-1 pl-5">
            <li>Put the card in the device and power it on.</li>
            <li>
              First boot takes a few minutes — it expands the filesystem, joins
              the network, then reboots once.
            </li>
            <li>
              If the WiFi you entered isn't found, the device starts its own
              hotspot named <span className="font-mono">{hostname}</span>{" "}
              (password <span className="font-mono">raspberry</span>). Join it
              and open <span className="font-mono">http://10.42.0.1</span> to
              set up the network.
            </li>
          </ol>
        </div>
        <Button type="button" size="lg" className="min-w-45" onClick={onReset}>
          Flash another
        </Button>
      </Pane>
    );
  }

  if (cancelled) {
    return (
      <Pane heading="Write">
        <IconBadge icon={OctagonX} tone="destructive" />
        <div className="flex flex-col gap-1.5">
          <h2 className="text-xl font-semibold">Stopped</h2>
          <p className="text-sm text-balance text-muted-foreground">
            You cancelled the write. The card is partly written and won't boot —
            flash it again before using it.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            size="lg"
            className="min-w-45"
            onClick={onRetry ?? undefined}
            disabled={onRetry === null}
          >
            Write again
          </Button>
          <Button
            type="button"
            size="lg"
            variant="secondary"
            onClick={onStartOver}
          >
            Change settings
          </Button>
        </div>
      </Pane>
    );
  }

  if (error) {
    return (
      <Pane heading="Write">
        <IconBadge icon={XCircle} tone="destructive" />
        <div className="flex flex-col gap-1.5">
          <h2 className="text-xl font-semibold">Flash failed</h2>
          <p className="text-sm text-balance text-muted-foreground">
            Something went wrong while writing the SD card.
          </p>
        </div>
        <Alert variant="destructive" className="text-left">
          <AlertTitle>Error details</AlertTitle>
          <AlertDescription className="break-words whitespace-pre-wrap">
            {error}
          </AlertDescription>
        </Alert>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            size="lg"
            className="min-w-45"
            onClick={onRetry ?? undefined}
            disabled={onRetry === null}
          >
            Try again
          </Button>
          <Button
            type="button"
            size="lg"
            variant="secondary"
            onClick={onStartOver}
          >
            Change settings
          </Button>
          <Button type="button" size="lg" variant="ghost" onClick={onRevealLog}>
            Show diagnostics
          </Button>
        </div>
      </Pane>
    );
  }

  if (progress.phase === "downloading") {
    const p = progress.progress;
    const percent = p ? Math.min(100, Math.round(p.percent)) : 0;
    const determinate = p !== null && p.total_bytes > 0;
    const speed = p ? formatSpeed(p.speed_bps) : "";
    const sizeDetail =
      p && p.total_bytes > 0
        ? `${formatBytes(p.downloaded_bytes)} / ${formatBytes(p.total_bytes)}`
        : null;
    const detail = [sizeDetail, speed].filter(Boolean).join(" · ") || null;
    return (
      <Pane heading="Writing">
        <ProgressPanel
          icon={HardDriveDownload}
          title="Downloading image…"
          subtitle="Fetching the OS image from Aircast."
          stepLabel="Download"
          percent={percent}
          detail={detail}
          determinate={determinate}
          onCancel={onCancel}
          showCancel
        />
      </Pane>
    );
  }

  if (progress.phase === "flashing") {
    const p = progress.progress;
    const phase = p?.phase;
    const percent = p ? Math.min(100, Math.round(p.percent)) : 0;
    const determinate =
      p !== null &&
      (p.phase === "writing" || p.phase === "verifying") &&
      p.total_bytes > 0;

    const title =
      phase === "decompressing"
        ? "Preparing image…"
        : phase === "verifying"
          ? "Verifying…"
          : phase === "customizing"
            ? "Applying settings…"
            : "Writing to SD card…";
    const subtitle =
      phase === "decompressing"
        ? "Decompressing the image before writing."
        : phase === "verifying"
          ? "Reading the card back to confirm a clean write."
          : phase === "customizing"
            ? "Writing your network and access settings to the card."
            : "Don't remove the SD card while writing.";
    const stepLabel =
      phase === "decompressing"
        ? "Prepare"
        : phase === "verifying"
          ? "Verify"
          : phase === "customizing"
            ? "Apply settings"
            : "Write";
    const sizeDetail =
      p &&
      (p.phase === "writing" || p.phase === "verifying") &&
      p.total_bytes > 0
        ? `${formatBytes(p.bytes_processed)} / ${formatBytes(p.total_bytes)}`
        : null;

    return (
      <Pane heading="Writing">
        <ProgressPanel
          icon={
            phase === "verifying" || phase === "customizing"
              ? ShieldCheck
              : HardDriveDownload
          }
          title={p ? title : "Preparing…"}
          subtitle={p ? subtitle : "Getting things ready."}
          stepLabel={stepLabel}
          percent={percent}
          detail={sizeDetail}
          determinate={determinate}
          onCancel={onCancel}
          showCancel
        />
      </Pane>
    );
  }

  return (
    <Pane heading="Writing">
      <ProgressPanel
        icon={Loader2}
        title="Starting…"
        subtitle="Preparing to flash your SD card."
        stepLabel={null}
        percent={0}
        detail={null}
        determinate={false}
        spinner
        onCancel={onCancel}
        showCancel
      />
    </Pane>
  );
}

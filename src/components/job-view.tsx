import {
  CheckCircle2,
  Download,
  HardDriveDownload,
  Loader2,
  OctagonX,
  ShieldCheck,
  XCircle,
  type LucideIcon,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { track } from "@/api";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { DeviceFinder } from "@/components/device-finder";

const QGC_DOWNLOAD_URL = "https://aircast.one/docs/software/qgroundcontrol";
import { Progress } from "@/components/ui/progress";
import { formatBytes, formatSpeed } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { FlashProgressState } from "@/components/wizard-types";
import { saveRecoveryKey } from "@/components/disk-encryption-section";
import { CloudSignInButton } from "@/components/cloud-cards-view";
import type { Board, EncryptionConfig } from "@/types";
import { useState } from "react";

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

function EncryptionReminder({
  hostname,
  encryption,
}: {
  hostname: string;
  encryption: EncryptionConfig;
}) {
  const [saved, setSaved] = useState(false);
  return (
    <div className="w-full rounded-xl border border-border px-4 py-3 text-left">
      <p className="flex items-center gap-1.5 text-sm font-medium">
        <ShieldCheck className="size-4 text-green-600 dark:text-green-400" />
        This card encrypts on first boot
      </p>
      <p className="mt-1 text-xs text-muted-foreground">
        Keep its recovery key — it's your only way in if the key servers are
        unreachable, and it isn't stored on the card.
      </p>
      <code className="mt-2 block break-all rounded bg-muted/50 px-2 py-1 text-xs">
        {encryption.recoveryKey}
      </code>
      <Button
        type="button"
        size="sm"
        variant="secondary"
        className="mt-2"
        onClick={() => {
          void saveRecoveryKey(
            encryption.tangServers,
            encryption.threshold,
            hostname,
            encryption.recoveryKey,
            false,
          )
            .then((ok) => setSaved(ok))
            .catch(() => setSaved(false));
        }}
      >
        <Download className="size-4" />
        {saved ? "Saved" : "Download recovery key"}
      </Button>
    </div>
  );
}

const BOARDS_DOCS_URL = "https://aircast.one/docs/hardware/boards";

function NotShowingUp({ board, noWifi }: { board: Board; noWifi: boolean }) {
  return (
    <div className="w-full text-left text-sm text-muted-foreground">
      <h3 className="mb-1 font-medium text-foreground">
        Not showing up after 10 minutes?
      </h3>
      <ul className="list-disc space-y-1 pl-5">
        {!board.builtinWifi && !noWifi ? (
          <li>
            {board.name} has no built-in WiFi. Plug in a USB WiFi adapter or an
            Ethernet cable, then power it on again.
          </li>
        ) : null}
        <li>
          This card holds the {board.name} image. It won't start on a
          different board.
        </li>
        <li>
          <button
            type="button"
            className="underline underline-offset-2 hover:text-foreground"
            onClick={() => void openUrl(BOARDS_DOCS_URL)}
          >
            Setup notes for each board
          </button>
        </li>
      </ul>
    </div>
  );
}

export function JobView({
  hostname,
  board,
  noWifi,
  remoteEnrolled,
  encryption,
  success,
  cancelled,
  error,
  cardTouched,
  signedOut,
  progress,
  onCancel,
  onRetry,
  onChangeSettings,
  onReset,
  onRevealLog,
}: {
  hostname: string;
  board: Board | null;
  noWifi: boolean;
  remoteEnrolled: boolean;
  encryption: EncryptionConfig | null;
  success: boolean;
  cancelled: boolean;
  error: string | null;
  cardTouched: boolean;
  signedOut: boolean;
  progress: FlashProgressState;
  onCancel: () => void;
  onRetry: (() => void) | null;
  onChangeSettings: () => void;
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
        <div className="w-full text-left text-sm text-muted-foreground">
          <h3 className="mb-1 font-medium text-foreground">What happens next</h3>
          <ol className="list-decimal space-y-1 pl-5">
            <li>Put the card in the device and power it on.</li>
            <li>
              Wait a few minutes. It restarts once on its own — don't unplug
              it.
            </li>
            <li>
              {remoteEnrolled
                ? "It appears below when it's ready — on this network or your private one."
                : "Keep this computer on the same network. The device appears below when it's ready."}
            </li>
          </ol>
        </div>
        {encryption ? (
          <EncryptionReminder hostname={hostname} encryption={encryption} />
        ) : null}
        <DeviceFinder hostname={hostname} remoteEnrolled={remoteEnrolled} />
        {board ? <NotShowingUp board={board} noWifi={noWifi} /> : null}
        <div className="flex w-full items-center justify-between gap-3 rounded-xl border border-border px-4 py-3 text-left">
          <div className="flex min-w-0 flex-col gap-0.5">
            <span className="text-sm font-medium">
              Aircast QGC{" "}
              <span className="font-normal text-muted-foreground">
                (optional)
              </span>
            </span>
            <span className="text-xs text-muted-foreground">
              QGroundControl with Aircast's low-latency video built in.
            </span>
          </div>
          <Button
            type="button"
            size="sm"
            variant="secondary"
            className="shrink-0"
            onClick={() => {
              track("qgc_download_click");
              void openUrl(QGC_DOWNLOAD_URL);
            }}
          >
            <Download className="size-4" />
            Get it
          </Button>
        </div>
        <Button
          type="button"
          size="lg"
          variant="secondary"
          className="min-w-45"
          onClick={onReset}
        >
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
            {cardTouched
              ? "You cancelled the write. The card is partly written and won't boot, so flash it again before using it."
              : "You cancelled before the write started. Nothing was written to the SD card."}
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
            onClick={onChangeSettings}
          >
            Change settings
          </Button>
        </div>
      </Pane>
    );
  }

  if (error && signedOut) {
    return (
      <Pane heading="Write">
        <IconBadge icon={XCircle} tone="destructive" />
        <div className="flex flex-col gap-1.5">
          <h2 className="text-xl font-semibold">Sign in to Aircast Cloud</h2>
          <p className="text-sm text-balance text-muted-foreground">
            Your Aircast Cloud sign-in ended before the write started, so
            nothing was written to the SD card. Sign in and the write starts
            again.
          </p>
        </div>
        <div className="flex items-start gap-2">
          <CloudSignInButton size="lg" onSignedIn={onRetry ?? undefined} />
          <Button
            type="button"
            size="lg"
            variant="secondary"
            onClick={onChangeSettings}
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
            {cardTouched
              ? "Something went wrong while writing the SD card. It is partly written and won't boot, so flash it again before using it."
              : "Something went wrong before the write started. Nothing was written to the SD card."}
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
            onClick={onChangeSettings}
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
    const rate = p && p.speed_bps > 0 ? formatSpeed(p.speed_bps) : null;
    const flashDetail = [sizeDetail, rate].filter(Boolean).join(" · ") || null;

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
          detail={flashDetail}
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

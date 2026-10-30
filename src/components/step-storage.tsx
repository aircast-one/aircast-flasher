import { AlertTriangle, Cpu, HardDrive, LoaderCircle } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { SelectableCard } from "@/components/selectable-card";
import { StepShell, type Problem } from "@/components/step-shell";
import { cn } from "@/lib/utils";
import type { SummaryItem } from "@/components/wizard-summary";
import { STEP, type WizardStep } from "@/components/wizard-types";
import { fitsBoard, isUsbTarget, usbFamily } from "@/components/usb-target";
import type { BlockDevice, Board } from "@/types";

export type { SummaryItem };

function FlashingModeSteps({
  board,
  running,
}: {
  board: Board;
  running: boolean;
}) {
  return (
    <div className="flex flex-col gap-3 rounded-xl border border-border p-4">
      <h2 className="text-sm font-medium">
        {running
          ? `Your ${board.maker} board is connected, but it's running its own system`
          : "No SD card? Write straight to the board over USB"}
      </h2>
      <ol className="list-decimal space-y-1 pl-5 text-sm text-muted-foreground">
        <li>Unplug the {board.name} from this computer.</li>
        <li>
          Hold its BOOT button (some boards label it MASKROM or RECOVERY).
        </li>
        <li>
          While holding it, plug the board&apos;s USB OTG port (USB-C on most
          boards) into this computer, then let go after 2 seconds.
        </li>
      </ol>
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <LoaderCircle className="size-4 animate-spin" />
        Waiting for the {board.name} in flashing mode…
      </p>
    </div>
  );
}

export function StepStorage({
  board,
  runningBoards,
  devices,
  devicesLoading,
  selectedDisk,
  onSelectDisk,
  requiredCardBytes,
  summary,
  problems,
  onBack,
  onEditStep,
  onFlash,
}: {
  board: Board | null;
  runningBoards: string[];
  devices: BlockDevice[];
  devicesLoading: boolean;
  selectedDisk: string;
  onSelectDisk: (path: string) => void;
  requiredCardBytes: number | undefined;
  summary: SummaryItem[];
  problems: Problem[];
  onBack: () => void;
  onEditStep: (step: WizardStep) => void;
  onFlash: () => void;
}) {
  const selected = devices.find((d) => d.path === selectedDisk) ?? null;
  const tooSmall = (d: BlockDevice) =>
    requiredCardBytes !== undefined && d.size > 0 && d.size < requiredCardBytes;
  const writesBoard = selected !== null && isUsbTarget(selected.path);
  const boardName = board?.name ?? "board";
  const usbCapable = usbFamily(board) !== null;
  const boardOnUsb = devices.some(
    (d) => isUsbTarget(d.path) && fitsBoard(d, board),
  );
  const titleOf = (d: BlockDevice) =>
    isUsbTarget(d.path) && fitsBoard(d, board) && board
      ? `${board.name}, built-in storage`
      : d.name;

  const rows: SummaryItem[] = [
    ...summary,
    {
      label: "Target",
      value: selected
        ? writesBoard
          ? `${boardName}, built-in storage (USB)`
          : `${selected.name} · ${selected.size_human}`
        : "Nothing selected yet",
      step: STEP.storage,
    },
  ];

  return (
    <StepShell
      heading="Storage"
      description={
        usbCapable
          ? "Insert an SD card, or connect the board in flashing mode. It shows up here on its own. Check the settings below, then write."
          : "Insert your SD card and it'll show up here on its own. Check the settings below, then write."
      }
      back={{ onClick: onBack }}
      next={{
        label: writesBoard ? `Write to ${boardName}` : "Flash SD Card",
        onClick: onFlash,
        problems,
      }}
    >
      <div className="flex flex-col gap-4">
        {devices.length === 0 ? (
          <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border py-14 text-center">
            <HardDrive className="size-8 text-muted-foreground/60" />
            <p className="text-sm text-muted-foreground">
              {devicesLoading
                ? "Scanning for storage devices…"
                : usbCapable
                  ? "Insert an SD card, or connect the board in flashing mode, and it'll show up here."
                  : "Insert an SD card and it'll show up here."}
            </p>
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            {devices.map((d) => (
              <SelectableCard
                key={d.path}
                icon={isUsbTarget(d.path) ? <Cpu /> : <HardDrive />}
                title={titleOf(d)}
                selected={selectedDisk === d.path}
                disabled={tooSmall(d) || !fitsBoard(d, board)}
                onClick={() => onSelectDisk(d.path)}
              >
                <span className="text-sm text-muted-foreground">
                  {d.size_human}
                </span>
                {isUsbTarget(d.path) && !fitsBoard(d, board) ? (
                  <span className="text-xs text-destructive">
                    This is a {d.name.split(" ")[0]} board; the image is for the{" "}
                    {boardName}
                  </span>
                ) : null}
                {tooSmall(d) ? (
                  <span className="text-xs text-destructive">
                    Too small for this image
                  </span>
                ) : null}
                {d.mounted && d.mount_points.length > 0 ? (
                  <span className="text-xs text-muted-foreground">
                    Mounted as {d.mount_points.join(", ")}
                  </span>
                ) : null}
              </SelectableCard>
            ))}
            {selected === null ? (
              <p className="text-sm text-muted-foreground">
                Pick where to write.
              </p>
            ) : null}
          </div>
        )}

        {board !== null && usbCapable && !boardOnUsb ? (
          <FlashingModeSteps board={board} running={runningBoards.length > 0} />
        ) : null}

        <div className="flex flex-col gap-3 rounded-xl border border-border p-4">
          <h2 className="text-sm font-medium">What will be written</h2>
          <dl className="flex flex-col gap-2">
            {rows.map(({ label, value, warn, step }) => (
              <div
                key={label}
                className="flex items-baseline justify-between gap-4 text-sm"
              >
                <dt className="shrink-0 text-muted-foreground">{label}</dt>
                <dd className="flex min-w-0 items-baseline gap-3">
                  <span
                    className={cn(
                      "text-right break-all",
                      warn && "font-medium text-destructive",
                    )}
                  >
                    {value}
                  </span>
                  {step === STEP.storage ? null : (
                    <button
                      type="button"
                      onClick={() => onEditStep(step)}
                      aria-label={`Change ${label}`}
                      className="shrink-0 rounded-sm text-muted-foreground underline underline-offset-2 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
                    >
                      Change
                    </button>
                  )}
                </dd>
              </div>
            ))}
          </dl>
        </div>

        {selected && writesBoard ? (
          <Alert variant="destructive">
            <AlertTriangle />
            <AlertTitle>
              Replace everything on the {boardName}&apos;s built-in storage?
            </AlertTitle>
            <AlertDescription>
              Whatever system is on it now is erased. This cannot be undone.
              Keep the board plugged in until the write finishes.
            </AlertDescription>
          </Alert>
        ) : selected ? (
          <Alert variant="destructive">
            <AlertTriangle />
            <AlertTitle>
              Erase {selected.name} · {selected.size_human}?
            </AlertTitle>
            <AlertDescription>
              Everything on {selected.path} is overwritten. This cannot be
              undone. You'll be asked for your password before the write starts
              — stay nearby.
            </AlertDescription>
          </Alert>
        ) : null}
      </div>
    </StepShell>
  );
}

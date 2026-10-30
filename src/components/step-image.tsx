import { useState } from "react";
import { Cpu, FileImage, Package, TriangleAlert } from "lucide-react";

import { SelectableCard } from "@/components/selectable-card";
import { StepShell, type Problem } from "@/components/step-shell";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { formatBytes, formatDate } from "@/lib/format";
import type { SourceKind } from "@/components/wizard-types";
import type { Board, Release } from "@/types";
import { releaseLabel } from "./release-label";

function aircastTitle(board: Board | null, release: Release | null): string {
  if (board === null) return "Aircast OS";
  const recommended = release !== null && !release.prerelease;
  return `Aircast OS for ${board.name}${recommended ? " (recommended)" : ""}`;
}

function BetaNote({ board }: { board: Board }) {
  return (
    <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2">
      <TriangleAlert className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400" />
      <p className="text-sm text-muted-foreground">
        {board.name} support is in beta: the image is built and checked, but
        hasn't been tested on real hardware yet.
      </p>
    </div>
  );
}

export function StepImage({
  boards,
  board,
  onBoard,
  sourceKind,
  onSourceKind,
  release,
  releases,
  onSelectVersion,
  releaseLoading,
  releaseError,
  localFileName,
  onPickLocal,
  cacheStatus,
  onNext,
}: {
  boards: Board[];
  board: Board | null;
  onBoard: (board: Board) => void;
  sourceKind: SourceKind;
  onSourceKind: (kind: SourceKind) => void;
  release: Release | null;
  releases: Release[];
  onSelectVersion: (version: string) => void;
  releaseLoading: boolean;
  releaseError: string | null;
  localFileName: string | null;
  onPickLocal: () => void;
  cacheStatus: string | null;
  onNext: () => void;
}) {
  const [pickedMaker, setPickedMaker] = useState<string | null>(null);
  const makers = [...new Set(boards.map((option) => option.maker))];
  const maker = pickedMaker ?? board?.maker ?? null;
  const models = boards.filter((option) => option.maker === maker);
  const chosen = board !== null && board.maker === maker ? board : null;
  const pickMaker = (name: string) => {
    const only = boards.filter((option) => option.maker === name);
    setPickedMaker(name);
    return only.length === 1 ? onBoard(only[0]) : undefined;
  };
  const pickModel = (id: string) => {
    const next = models.find((option) => option.id === id);
    return next ? onBoard(next) : undefined;
  };

  const problems: Problem[] =
    sourceKind === "local"
      ? localFileName === null
        ? [{ message: "Choose the image file to write", fix: onPickLocal }]
        : []
      : chosen === null
        ? [
            {
              message:
                maker !== null && models.length > 1
                  ? `Choose the ${maker} model`
                  : "Choose the board this card is for",
            },
          ]
        : release !== null
          ? []
          : releaseLoading
            ? [{ message: "Wait for the Aircast OS releases to load" }]
            : [
                {
                  message:
                    "Aircast OS isn't available right now — choose a local image file instead",
                  fix: onPickLocal,
                },
              ];

  return (
    <StepShell
      heading="Operating system"
      description="Choose the board this card is for, then the image to write."
      next={{ label: "Next", onClick: onNext, problems }}
    >
      <div className="flex flex-col gap-6">
        <div className="flex flex-col gap-3">
          <h2 className="text-sm font-medium">Which board is this card for?</h2>
          <div
            role="group"
            aria-label="Board"
            className="grid grid-cols-3 gap-3"
          >
            {makers.map((name) => {
              const range = boards.filter((option) => option.maker === name);
              return (
                <SelectableCard
                  key={name}
                  icon={<Cpu />}
                  title={
                    <span className="flex flex-wrap items-center gap-x-2">
                      {name}
                      {range.every((option) => option.beta) ? (
                        <span className="rounded-full bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground">
                          Beta
                        </span>
                      ) : null}
                    </span>
                  }
                  selected={maker === name}
                  onClick={() => pickMaker(name)}
                >
                  <span className="text-sm text-muted-foreground">
                    {range.length === 1
                      ? range[0].detail
                      : `${range.length} models`}
                  </span>
                </SelectableCard>
              );
            })}
          </div>
          {models.length > 1 ? (
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <span className="text-sm text-muted-foreground">Model</span>
              <Select
                value={chosen ? chosen.id : ""}
                onValueChange={(value) => pickModel(String(value))}
              >
                <SelectTrigger
                  size="sm"
                  aria-label="Model"
                  className="min-w-56"
                >
                  <SelectValue>
                    {(value) =>
                      models.find((option) => option.id === value)?.name ??
                      `Choose the ${maker} model`
                    }
                  </SelectValue>
                </SelectTrigger>
                <SelectContent className="min-w-72">
                  {models.map((option) => (
                    <SelectItem key={option.id} value={option.id}>
                      <span className="flex-1">{option.name}</span>
                      <span className="text-xs text-muted-foreground">
                        {option.builtinWifi ? "WiFi" : "no WiFi"}
                      </span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {chosen ? (
                <span className="text-sm text-muted-foreground">
                  {chosen.detail} ·{" "}
                  {chosen.builtinWifi ? "built-in WiFi" : "no built-in WiFi"}
                </span>
              ) : null}
            </div>
          ) : null}
        </div>

        <div className="flex flex-col gap-3">
          <h2 className="text-sm font-medium">Image</h2>
          <SelectableCard
            icon={<Package />}
            title={aircastTitle(chosen, release)}
            selected={sourceKind === "aircast"}
            onClick={() => onSourceKind("aircast")}
          >
            <span className="text-sm break-words text-muted-foreground">
              {chosen === null
                ? "Choose a board to see its releases"
                : releaseLoading
                  ? "Loading…"
                  : releaseError
                    ? releaseError
                    : release
                      ? `${release.version}${releaseLabel(release)} · ${release.image.filename} · ${formatBytes(release.image.size)}`
                      : "No release"}
            </span>
            {sourceKind === "aircast" && cacheStatus ? (
              <span className="mt-1 block text-xs text-muted-foreground">
                {cacheStatus}
              </span>
            ) : null}
          </SelectableCard>

          {sourceKind === "aircast" && chosen?.beta ? (
            <BetaNote board={chosen} />
          ) : null}

          {sourceKind === "aircast" &&
            chosen !== null &&
            releases.length > 1 && (
              <div className="flex items-center gap-2 pl-14">
                <span className="text-sm text-muted-foreground">Version</span>
                <Select
                  value={release ? release.version : ""}
                  onValueChange={(value) => onSelectVersion(String(value))}
                >
                  <SelectTrigger size="sm" aria-label="Aircast OS version">
                    <SelectValue>{(value) => String(value)}</SelectValue>
                  </SelectTrigger>
                  <SelectContent className="min-w-64">
                    {releases.map((option, index) => (
                      <SelectItem key={option.version} value={option.version}>
                        <span className="flex-1">
                          {option.version}
                          {index === 0 ? " (latest)" : ""}
                          {releaseLabel(option)}
                        </span>
                        <span className="text-xs text-muted-foreground">
                          {formatDate(option.created_at)}
                        </span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

          <SelectableCard
            icon={<FileImage />}
            title="Use custom .img"
            selected={sourceKind === "local"}
            onClick={onPickLocal}
          >
            <span className="text-sm break-words text-muted-foreground">
              {sourceKind === "local" && localFileName
                ? localFileName
                : "Browse for a local .img, .xz or .gz file"}
            </span>
          </SelectableCard>
        </div>
      </div>
    </StepShell>
  );
}

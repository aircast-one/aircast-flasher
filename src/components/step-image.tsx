import { FileImage, Package } from "lucide-react";

import { SelectableCard } from "@/components/selectable-card";
import { StepShell } from "@/components/step-shell";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { formatBytes, formatDate } from "@/lib/format";
import type { SourceKind } from "@/components/wizard-types";
import type { Release } from "@/types";
import { releaseLabel } from "./release-label";

export function StepImage({
  sourceKind,
  onSourceKind,
  release,
  releases,
  onSelectVersion,
  releaseLoading,
  releaseError,
  localFileName,
  onPickLocal,
  canProceed,
  onNext,
}: {
  sourceKind: SourceKind;
  onSourceKind: (kind: SourceKind) => void;
  release: Release | null;
  releases: Release[];
  onSelectVersion: (version: string) => void;
  releaseLoading: boolean;
  releaseError: string | null;
  localFileName: string | null;
  onPickLocal: () => void;
  canProceed: boolean;
  onNext: () => void;
}) {
  return (
    <StepShell
      heading="Operating system"
      description="Pick the image to write to the card."
      next={{ label: "Next", onClick: onNext, disabled: !canProceed }}
    >
      <div className="flex flex-col gap-3">
        <SelectableCard
          icon={<Package />}
          title="Aircast OS (recommended)"
          selected={sourceKind === "aircast"}
          onClick={() => onSourceKind("aircast")}
        >
          <span className="text-sm break-words text-muted-foreground">
            {releaseLoading
              ? "Loading…"
              : releaseError
                ? releaseError
                : release
                  ? `${release.version} · ${release.image.filename} · ${formatBytes(release.image.size)}`
                  : "No release"}
          </span>
        </SelectableCard>

        {sourceKind === "aircast" && releases.length > 1 && (
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
    </StepShell>
  );
}

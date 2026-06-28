import { FileImage, Package } from "lucide-react";

import { SelectableCard } from "@/components/selectable-card";
import { StepShell } from "@/components/step-shell";
import { formatBytes } from "@/lib/format";
import type { SourceKind } from "@/components/wizard-types";
import type { Release } from "@/types";

export function StepImage({
  sourceKind,
  onSourceKind,
  release,
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
  releaseLoading: boolean;
  releaseError: string | null;
  localFileName: string | null;
  onPickLocal: () => void;
  canProceed: boolean;
  onNext: () => void;
}) {
  return (
    <StepShell
      heading="Choose the image"
      description="Pick the operating system image to write."
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

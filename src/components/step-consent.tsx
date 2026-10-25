import { BarChart3 } from "lucide-react";

import { SettingCard } from "@/components/setting-card";
import { StepShell } from "@/components/step-shell";

export function StepConsent({
  onAnswer,
}: {
  onAnswer: (share: boolean) => void;
}) {
  return (
    <StepShell
      heading="Diagnostics"
      description="One question before your first card. You can change it any time from Diagnostics in the sidebar."
      back={{ label: "No thanks", onClick: () => onAnswer(false) }}
      next={{ label: "Share", onClick: () => onAnswer(true) }}
    >
      <div data-testid="consent-step">
        <SettingCard
          icon={<BarChart3 />}
          title="Share anonymous diagnostics"
          description="Which step failed and how fast the card wrote. It never includes your network name, passphrase, keys, or hostname."
          action={null}
        />
      </div>
    </StepShell>
  );
}

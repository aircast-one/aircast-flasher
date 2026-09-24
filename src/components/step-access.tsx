import { useState } from "react";

import { StepShell } from "@/components/step-shell";
import { CollapsibleSection } from "@/components/collapsible-section";
import {
  RemoteAccessSection,
  type RemoteAccessProps,
} from "@/components/remote-access-section";
import {
  DeviceAccessSection,
  type DeviceAccessProps,
} from "@/components/device-access-section";
import {
  isInvalidSshKey,
  isValidControlServer,
  isWeakDevicePassword,
  needsControlServer,
} from "@/components/step-network.validation";

type RemoteAccessInputs = Pick<
  RemoteAccessProps,
  "controlServer" | "onControlServer" | "authKey" | "onAuthKey"
>;

type DeviceAccessInputs = Pick<
  DeviceAccessProps,
  | "sshMode"
  | "onSshMode"
  | "sshKey"
  | "onSshKey"
  | "detectedKeys"
  | "onChooseKeyFile"
  | "devicePassword"
  | "onDevicePassword"
>;

export function StepAccess({
  remote,
  access,
  onBack,
  onNext,
}: {
  remote: RemoteAccessInputs;
  access: DeviceAccessInputs;
  onBack: () => void;
  onNext: () => void;
}) {
  const [selfHosted, setSelfHosted] = useState(
    () => remote.controlServer.trim() !== "",
  );

  const controlServerValid = isValidControlServer(remote.controlServer);
  const controlServerMissing = needsControlServer(
    selfHosted,
    remote.controlServer,
    remote.authKey,
  );
  const remoteAccessValid = controlServerValid && !controlServerMissing;

  const sshKeyInvalid =
    access.sshMode === "key-only" && isInvalidSshKey(access.sshKey);
  const devicePasswordWeak =
    access.sshMode === "password" &&
    isWeakDevicePassword(access.devicePassword);

  return (
    <StepShell
      heading="Access"
      description="How you reach this device once it's running. Both are optional — skip and the image defaults apply."
      back={{ onClick: onBack }}
      next={{
        label: "Next",
        onClick: onNext,
        disabled: sshKeyInvalid || devicePasswordWeak,
      }}
    >
      <div className="flex max-w-xl flex-col gap-5">
        <CollapsibleSection
          title="Remote access (optional)"
          forceOpen={!remoteAccessValid}
          initiallyOpen={remote.authKey.trim() !== ""}
        >
          <RemoteAccessSection
            {...remote}
            selfHosted={selfHosted}
            onSelfHosted={setSelfHosted}
            controlServerValid={controlServerValid}
            controlServerMissing={controlServerMissing}
          />
        </CollapsibleSection>

        <CollapsibleSection
          title="Device access"
          forceOpen={sshKeyInvalid || devicePasswordWeak}
          initiallyOpen
        >
          <DeviceAccessSection
            {...access}
            sshKeyInvalid={sshKeyInvalid}
            devicePasswordWeak={devicePasswordWeak}
          />
        </CollapsibleSection>
      </div>
    </StepShell>
  );
}

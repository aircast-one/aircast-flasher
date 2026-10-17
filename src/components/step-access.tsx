import { useState } from "react";
import { Globe, Lock, SquareTerminal } from "lucide-react";

import { StepShell } from "@/components/step-shell";
import { SettingCard } from "@/components/setting-card";
import { Switch } from "@/components/ui/switch";
import {
  RemoteAccessSection,
  type RemoteAccessProps,
} from "@/components/remote-access-section";
import {
  DeviceAccessSection,
  SshModePicker,
  type DeviceAccessProps,
} from "@/components/device-access-section";
import {
  DiskEncryptionSection,
  type DiskEncryptionProps,
} from "@/components/disk-encryption-section";
import { encryptionIncomplete } from "@/components/encryption";
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
  encryption,
  onBack,
  onNext,
}: {
  remote: RemoteAccessInputs;
  access: DeviceAccessInputs;
  encryption: DiskEncryptionProps;
  onBack: () => void;
  onNext: () => void;
}) {
  const [selfHosted, setSelfHosted] = useState(
    () => remote.controlServer.trim() !== "",
  );
  const [remoteOn, setRemoteOn] = useState(() => remote.authKey.trim() !== "");

  const controlServerValid = isValidControlServer(remote.controlServer);
  const controlServerMissing = needsControlServer(
    selfHosted,
    remote.controlServer,
    remote.authKey,
  );

  const sshKeyInvalid =
    access.sshMode === "key-only" && isInvalidSshKey(access.sshKey);
  const devicePasswordWeak =
    access.sshMode === "password" &&
    isWeakDevicePassword(access.devicePassword);
  const encryptionPending = encryptionIncomplete(encryption);

  const toggleRemote = (on: boolean) => {
    setRemoteOn(on);
    if (!on) remote.onAuthKey("");
  };

  return (
    <StepShell
      heading="Access"
      description="How you'll get into this drone once it's running."
      back={{ onClick: onBack }}
      next={{
        label: "Next",
        onClick: onNext,
        disabled: sshKeyInvalid || devicePasswordWeak || encryptionPending,
      }}
    >
      <div className="flex max-w-xl flex-col gap-4">
        <SettingCard
          icon={<SquareTerminal />}
          title="SSH login"
          description="Log in to the drone's shell"
          action={
            <SshModePicker value={access.sshMode} onChange={access.onSshMode} />
          }
        >
          <DeviceAccessSection
            {...access}
            sshKeyInvalid={sshKeyInvalid}
            devicePasswordWeak={devicePasswordWeak}
          />
        </SettingCard>

        <SettingCard
          icon={<Globe />}
          title="Remote access"
          description={`Reach it from anywhere over ${selfHosted ? "Headscale" : "Tailscale"}`}
          action={
            <Switch
              aria-label="Remote access"
              checked={remoteOn}
              onCheckedChange={toggleRemote}
            />
          }
        >
          {remoteOn ? (
            <RemoteAccessSection
              {...remote}
              selfHosted={selfHosted}
              onSelfHosted={setSelfHosted}
              controlServerValid={controlServerValid}
              controlServerMissing={controlServerMissing}
            />
          ) : null}
        </SettingCard>

        <SettingCard
          icon={<Lock />}
          title="Disk encryption"
          description="A pulled card gives nothing away"
          action={
            <Switch
              aria-label="Disk encryption"
              checked={encryption.enabled}
              onCheckedChange={encryption.onEnabled}
            />
          }
        >
          {encryption.enabled ? <DiskEncryptionSection {...encryption} /> : null}
        </SettingCard>
      </div>
    </StepShell>
  );
}

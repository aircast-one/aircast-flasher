import { useState } from "react";
import { Globe, Lock, SquareTerminal, Swords } from "lucide-react";

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
import {
  ENCRYPTION_MIN_IMAGE,
  encryptionIncomplete,
} from "@/components/encryption";
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
  encryptionAvailable,
  fight,
  onBack,
  onNext,
}: {
  remote: RemoteAccessInputs;
  access: DeviceAccessInputs;
  encryption: DiskEncryptionProps;
  encryptionAvailable: boolean;
  fight: { enabled: boolean; onEnabled: (v: boolean) => void };
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
            <SshModePicker
              value={access.sshMode}
              onChange={access.onSshMode}
              keysOnly={fight.enabled}
            />
          }
        >
          <DeviceAccessSection
            {...access}
            sshKeyInvalid={sshKeyInvalid}
            devicePasswordWeak={devicePasswordWeak}
            keysOnly={fight.enabled}
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
          description={
            encryptionAvailable
              ? "A pulled card gives nothing away"
              : `Needs Aircast OS v${ENCRYPTION_MIN_IMAGE} or newer — pick it in Operating system`
          }
          action={
            <Switch
              aria-label="Disk encryption"
              disabled={!encryptionAvailable}
              checked={encryption.enabled}
              onCheckedChange={encryption.onEnabled}
            />
          }
        >
          {encryption.enabled ? <DiskEncryptionSection {...encryption} /> : null}
        </SettingCard>

        <SettingCard
          icon={<Swords />}
          title="Fight mode"
          description="No logs on the drone or on this computer"
          action={
            <Switch
              aria-label="Fight mode"
              checked={fight.enabled}
              onCheckedChange={fight.onEnabled}
            />
          }
        >
          {fight.enabled ? (
            <>
              <p className="text-sm text-muted-foreground">
                The drone records no video and wipes its flight log and journal
                on every start, along with the flight controller's logs. Only
                the tailnet gets in, and only with an SSH key. This computer
                forgets the WiFi password, SSH key and drone names.
              </p>
              {remote.authKey.trim() === "" ? (
                <p className="text-sm text-amber-600 dark:text-amber-400">
                  Remote access is off, so nothing can connect to this drone.
                </p>
              ) : null}
            </>
          ) : null}
        </SettingCard>
      </div>
    </StepShell>
  );
}

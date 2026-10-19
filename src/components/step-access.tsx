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
  fightModeUnreachable,
  isInvalidSshKey,
  isValidControlServer,
  isWeakDevicePassword,
  needsControlServer,
} from "@/components/step-network.validation";

type RemoteAccessInputs = Pick<
  RemoteAccessProps,
  | "controlServer"
  | "onControlServer"
  | "authKey"
  | "onAuthKey"
  | "selfHosted"
  | "onSelfHosted"
> & { enabled: boolean; onEnabled: (v: boolean) => void };

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
  const { enabled: remoteOn, onEnabled: onRemoteOn, ...remoteFields } = remote;
  const tailnetKey = remoteOn ? remote.authKey : "";
  const tailnetServer = remote.selfHosted ? remote.controlServer : "";

  const controlServerValid = isValidControlServer(remote.controlServer);
  const controlServerMissing = needsControlServer(
    remote.selfHosted,
    remote.controlServer,
    tailnetKey,
  );

  const sshKeyInvalid =
    access.sshMode === "key-only" && isInvalidSshKey(access.sshKey);
  const devicePasswordWeak =
    access.sshMode === "password" &&
    isWeakDevicePassword(access.devicePassword);
  const encryptionPending = encryptionIncomplete(encryption);
  const unreachable =
    fightModeUnreachable(fight.enabled, tailnetKey, tailnetServer) ||
    (fight.enabled && controlServerMissing);

  return (
    <StepShell
      heading="Access"
      description="How you'll get into this drone once it's running."
      back={{ onClick: onBack }}
      next={{
        label: "Next",
        onClick: onNext,
        disabled:
          sshKeyInvalid ||
          devicePasswordWeak ||
          encryptionPending ||
          unreachable,
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
          description={
            fight.enabled
              ? "The only way in while fight mode is on"
              : `Reach it from anywhere over ${remote.selfHosted ? "Headscale" : "Tailscale"}`
          }
          action={
            <Switch
              aria-label="Remote access"
              checked={remoteOn}
              disabled={fight.enabled}
              onCheckedChange={onRemoteOn}
            />
          }
        >
          {remoteOn ? (
            <RemoteAccessSection
              {...remoteFields}
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
          description="No logs on the drone"
          action={
            <Switch
              aria-label="Fight mode"
              checked={fight.enabled}
              onCheckedChange={fight.onEnabled}
            />
          }
        >
          {fight.enabled ? (
            <div className="flex flex-col gap-2 text-sm text-muted-foreground">
              <p>
                No video is recorded, and the flight log, journal and flight
                controller logs are wiped on every start. Only the tailnet gets
                in, and only with an SSH key.
              </p>
              {unreachable ? (
                <p id="fight-mode-help" className="text-destructive">
                  Finish Remote access above to continue. In fight mode the
                  tailnet is the only way into this drone.
                </p>
              ) : null}
            </div>
          ) : null}
        </SettingCard>
      </div>
    </StepShell>
  );
}

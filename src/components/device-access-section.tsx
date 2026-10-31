import { useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { Eye, EyeOff, FileKey } from "lucide-react";

import { identifyPublicKey } from "@/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Segmented } from "@/components/segmented";
import { minDevicePassword } from "@/components/step-network.validation";
import { cn } from "@/lib/utils";
import type { SshKeyIdentity, SshMode, SshPublicKey } from "@/types";

const SSH_MODES: { value: SshMode; label: string }[] = [
  { value: "key-only", label: "SSH key" },
  { value: "password", label: "Password" },
  { value: "disabled", label: "Off" },
];

const OTHER_KEY = "other";

export function SshModePicker({
  value,
  onChange,
}: {
  value: SshMode;
  onChange: (v: SshMode) => void;
}) {
  return (
    <Segmented
      label="SSH login"
      options={SSH_MODES}
      value={value}
      onChange={onChange}
    />
  );
}

function useKeyIdentity(sshKey: string): SshKeyIdentity | null {
  const key = sshKey.trim();
  const { data } = useQuery({
    queryKey: ["ssh-key-identity", key],
    queryFn: () => identifyPublicKey(key),
    enabled: key !== "",
    retry: false,
    staleTime: Infinity,
  });
  return key === "" ? null : (data ?? null);
}

function Hint({
  id,
  tone = "muted",
  children,
}: {
  id?: string;
  tone?: "muted" | "warn" | "error";
  children: ReactNode;
}) {
  return (
    <p
      id={id}
      className={cn(
        "text-sm",
        tone === "muted" && "text-muted-foreground",
        tone === "warn" && "text-amber-600 dark:text-amber-400",
        tone === "error" && "text-destructive",
      )}
    >
      {children}
    </p>
  );
}

function KeyOption({ sshKey: k }: { sshKey: SshPublicKey }) {
  const identity = useKeyIdentity(k.contents);
  const name = identity?.comment || k.label;

  return (
    <label className="flex cursor-pointer items-start gap-3 rounded-lg border border-border px-3 py-2.5 transition-colors hover:bg-muted/40 has-data-checked:border-primary has-data-checked:bg-primary/5">
      <RadioGroupItem value={k.label} className="mt-0.5" />
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-sm">
          <span className="font-medium">{name}</span>
          <span className="text-muted-foreground">
            {" · "}
            {k.label}
          </span>
        </span>
        {identity ? (
          <span className="truncate font-mono text-xs text-muted-foreground">
            {identity.fingerprint}
          </span>
        ) : null}
      </span>
    </label>
  );
}

function PastedKeyIdentity({ sshKey }: { sshKey: string }) {
  const identity = useKeyIdentity(sshKey);
  if (identity === null) return null;

  return (
    <div className="flex flex-col rounded-md bg-muted/50 px-2.5 py-1.5 text-xs">
      <span className="truncate">
        <span className="font-medium">
          {identity.comment === "" ? "No comment" : identity.comment}
        </span>
        <span className="text-muted-foreground">
          {" · "}
          {identity.algorithm}
        </span>
      </span>
      <span className="truncate font-mono text-muted-foreground">
        {identity.fingerprint}
      </span>
    </div>
  );
}

function PastedKey({
  sshKey,
  onSshKey,
  onChooseKeyFile,
  sshKeyInvalid,
}: {
  sshKey: string;
  onSshKey: (v: string) => void;
  onChooseKeyFile: () => void;
  sshKeyInvalid: boolean;
}) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-2">
        <Input
          id="ssh-key"
          aria-label="Public key"
          className="flex-1 font-mono"
          value={sshKey}
          onChange={(e) => onSshKey(e.currentTarget.value)}
          placeholder="Paste a key — ssh-ed25519 AAAA… you@host"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          aria-invalid={sshKeyInvalid}
          aria-describedby="ssh-key-help"
        />
        <Button type="button" variant="secondary" onClick={onChooseKeyFile}>
          <FileKey />
          Choose file…
        </Button>
      </div>
      <PastedKeyIdentity sshKey={sshKey} />
    </div>
  );
}

function KeyPicker({
  sshKey,
  onSshKey,
  detectedKeys,
  onChooseKeyFile,
  sshKeyInvalid,
}: {
  sshKey: string;
  onSshKey: (v: string) => void;
  detectedKeys: SshPublicKey[];
  onChooseKeyFile: () => void;
  sshKeyInvalid: boolean;
}) {
  const matched = detectedKeys.find(
    (k) => k.contents.trim() === sshKey.trim(),
  );
  const pasted = sshKey.trim() !== "" && matched === undefined;
  const [otherPicked, setOtherPicked] = useState(pasted);
  const other = otherPicked || pasted;

  const pick = (value: unknown) => {
    setOtherPicked(value === OTHER_KEY);
    onSshKey(detectedKeys.find((k) => k.label === value)?.contents ?? "");
  };

  const pastedKey = (
    <PastedKey
      sshKey={sshKey}
      onSshKey={onSshKey}
      onChooseKeyFile={onChooseKeyFile}
      sshKeyInvalid={sshKeyInvalid}
    />
  );

  if (detectedKeys.length === 0) return pastedKey;

  return (
    <div className="flex flex-col gap-2">
      <RadioGroup
        aria-label="Public key"
        value={other ? OTHER_KEY : (matched?.label ?? null)}
        onValueChange={pick}
      >
        {detectedKeys.map((k) => (
          <KeyOption key={k.label} sshKey={k} />
        ))}
        <label className="flex cursor-pointer items-center gap-3 rounded-lg border border-border px-3 py-2.5 text-sm font-medium transition-colors hover:bg-muted/40 has-data-checked:border-primary has-data-checked:bg-primary/5">
          <RadioGroupItem value={OTHER_KEY} />
          Another key
        </label>
      </RadioGroup>
      {other ? pastedKey : null}
    </div>
  );
}

export interface DeviceAccessProps {
  sshMode: SshMode;
  onSshMode: (v: SshMode) => void;
  sshKey: string;
  onSshKey: (v: string) => void;
  detectedKeys: SshPublicKey[];
  onChooseKeyFile: () => void;
  devicePassword: string;
  onDevicePassword: (v: string) => void;
  sshKeyInvalid: boolean;
  devicePasswordWeak: boolean;
  fighting: boolean;
}

export function DeviceAccessSection({
  sshMode,
  sshKey,
  onSshKey,
  detectedKeys,
  onChooseKeyFile,
  devicePassword,
  onDevicePassword,
  sshKeyInvalid,
  devicePasswordWeak,
  fighting,
}: DeviceAccessProps) {
  const [showDevicePassword, setShowDevicePassword] = useState(false);

  if (sshMode === "disabled") {
    return (
      <Hint>
        SSH is turned off. Manage this drone from its web dashboard instead.
      </Hint>
    );
  }

  if (sshMode === "password") {
    return (
      <>
        <div className="flex gap-2">
          <Input
            id="device-password"
            aria-label="Device password"
            type={showDevicePassword ? "text" : "password"}
            value={devicePassword}
            onChange={(e) => onDevicePassword(e.currentTarget.value)}
            placeholder="New password for the pi account"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            aria-invalid={devicePasswordWeak}
            aria-describedby="device-password-help"
            className="flex-1"
          />
          <Button
            type="button"
            variant="secondary"
            size="icon"
            onClick={() => setShowDevicePassword((v) => !v)}
            aria-label={showDevicePassword ? "Hide password" : "Show password"}
          >
            {showDevicePassword ? <EyeOff /> : <Eye />}
          </Button>
        </div>
        {devicePasswordWeak ? (
          <Hint id="device-password-help" tone="error">
            Use at least {minDevicePassword(fighting)} characters.
          </Hint>
        ) : devicePassword === "" ? (
          <Hint id="device-password-help" tone="warn">
            Blank keeps the default pi / raspberry login.
          </Hint>
        ) : (
          <Hint id="device-password-help">
            Replaces the default password on the pi account.
          </Hint>
        )}
      </>
    );
  }

  return (
    <>
      <KeyPicker
        sshKey={sshKey}
        onSshKey={onSshKey}
        detectedKeys={detectedKeys}
        onChooseKeyFile={onChooseKeyFile}
        sshKeyInvalid={sshKeyInvalid}
      />
      {sshKeyInvalid ? (
        <Hint id="ssh-key-help" tone="error">
          That doesn't look like an SSH public key — it should start with
          ssh-ed25519, ssh-rsa, …
        </Hint>
      ) : sshKey.trim() === "" ? (
        <Hint id="ssh-key-help" tone="warn">
          {fighting
            ? "No key yet — add one, pick Password, or turn SSH off. Fight mode won't leave the default login on."
            : "No key yet — the default pi / raspberry login stays on."}
        </Hint>
      ) : (
        <Hint id="ssh-key-help">
          Password login is turned off, so only this key gets in. Lose it and
          you'll need to re-flash.
        </Hint>
      )}
    </>
  );
}

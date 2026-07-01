import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  cancelFlash,
  detectSshKeys,
  downloadImage,
  flashImage,
  listBlockDevices,
  listReleases,
  listWifiNetworks,
  onDownloadProgress,
  onFlashProgress,
  pickAndReadPublicKey,
  pickLocalImage,
} from "@/api";
import type {
  AccessConfig,
  BlockDevice,
  InitFormat,
  Release,
  SshMode,
  TailscaleConfig,
  WifiConfig,
} from "@/types";
import { errorMessage } from "@/lib/format";
import { useLocalStorage } from "@/lib/use-local-storage";
import { UpdateBanner } from "@/components/update-banner";
import { WizardSidebar } from "@/components/wizard-sidebar";
import { StepStorage } from "@/components/step-storage";
import { StepImage } from "@/components/step-image";
import { StepNetwork } from "@/components/step-network";
import { JobView } from "@/components/job-view";
import {
  STEP,
  type FlashProgressState,
  type FlashVars,
  type SourceKind,
  type WizardStep,
} from "@/components/wizard-types";

const DEVICE_POLL_INTERVAL_MS = 2000;

function pickDefaultRelease(releases: Release[]): Release | null {
  return releases.find((r) => !r.prerelease) ?? releases[0] ?? null;
}

// Build the access config to send, or null when the chosen mode has no input
// yet (so the image's default pi/raspberry is left untouched). "disabled" always
// applies.
function buildAccess(
  mode: SshMode,
  sshKey: string,
  password: string,
): AccessConfig | null {
  if (mode === "disabled") {
    return { ssh: "disabled" };
  }
  if (mode === "key-only") {
    const key = sshKey.trim();
    return key === "" ? null : { ssh: "key-only", authorizedKey: key };
  }
  return password === "" ? null : { ssh: "password", password };
}

function App() {
  const queryClient = useQueryClient();

  // Image source
  const [sourceKind, setSourceKind] = useState<SourceKind>("aircast");
  const [localPath, setLocalPath] = useState<string | null>(null);

  // Drives
  const [selectedDisk, setSelectedDisk] = useState<string>("");

  // Network + identity
  // Persisted across launches so they don't have to be re-entered each flash.
  const [ssid, setSsid] = useLocalStorage("aircast.wifi.ssid", "");
  const [password, setPassword] = useLocalStorage("aircast.wifi.password", "");
  const [showPassword, setShowPassword] = useState(false);
  const [hostname, setHostname] = useLocalStorage("aircast.hostname", "");

  // Tailscale/Headscale enrollment. The control server is convenient to reuse
  // across flashes, but the pre-auth key is a secret — keep it in memory only.
  const [controlServer, setControlServer] = useLocalStorage(
    "aircast.tailscale.controlServer",
    "",
  );
  const [authKey, setAuthKey] = useState("");

  // Device access (SSH). Defaults to password auth with the image's stock
  // password prefilled, so a device is reachable out of the box; the operator
  // changes it (or switches to a key) for anything deployed. The public key is
  // reusable across flashes, so it's persisted; the password stays in memory.
  const [sshMode, setSshMode] = useState<SshMode>("password");
  const [sshKey, setSshKey] = useLocalStorage("aircast.ssh.authorizedKey", "");
  const [devicePassword, setDevicePassword] = useState("raspberry");

  // Wizard
  const [step, setStep] = useState<WizardStep>(STEP.os);

  // Live progress from Tauri events.
  const [progress, setProgress] = useState<FlashProgressState>({
    phase: "idle",
  });

  // Poll for storage devices only while the user is on the storage step, so an
  // inserted SD card is detected automatically without a manual refresh. The
  // query is disabled everywhere else (notably during the write) to keep
  // diskutil/lsblk off the target device and out of app startup.
  const onStorageStep = step === STEP.storage;
  const devicesQuery = useQuery({
    queryKey: ["devices"],
    queryFn: listBlockDevices,
    enabled: onStorageStep,
    refetchInterval: onStorageStep ? DEVICE_POLL_INTERVAL_MS : false,
    refetchOnWindowFocus: onStorageStep,
  });

  const releasesQuery = useQuery({
    queryKey: ["releases"],
    queryFn: () => listReleases(),
  });

  const wifiQuery = useQuery({
    queryKey: ["wifi"],
    queryFn: listWifiNetworks,
  });

  const sshKeysQuery = useQuery({
    queryKey: ["ssh-keys"],
    queryFn: detectSshKeys,
  });

  async function handleChooseKeyFile() {
    const contents = await pickAndReadPublicKey();
    if (contents) setSshKey(contents);
  }

  // Prefill the currently-joined network once, while the field is empty. The
  // WiFi country is detected on the backend and applied at flash time — never
  // shown to the user.
  useEffect(() => {
    const current = wifiQuery.data?.current;
    if (current) setSsid((prev) => (prev === "" ? current : prev));
  }, [wifiQuery.data]);

  const devices: BlockDevice[] = devicesQuery.data ?? [];
  const release = useMemo(
    () => (releasesQuery.data ? pickDefaultRelease(releasesQuery.data) : null),
    [releasesQuery.data],
  );
  const releaseError = releasesQuery.isError
    ? errorMessage(releasesQuery.error, "Failed to load releases.")
    : releasesQuery.isSuccess && !release
      ? "No releases available."
      : null;

  // Keep the selected disk valid as the device list changes. Depends on the
  // raw query data — not the `devices` array, which is a fresh reference each
  // render and would re-run this on every render until data loads.
  useEffect(() => {
    const list = devicesQuery.data ?? [];
    setSelectedDisk((prev) =>
      list.some((d) => d.path === prev) ? prev : (list[0]?.path ?? ""),
    );
  }, [devicesQuery.data]);

  // Subscribe to download/flash progress events for the lifetime of the app.
  useEffect(() => {
    let active = true;
    let unlistenDownload: (() => void) | undefined;
    let unlistenFlash: (() => void) | undefined;

    onDownloadProgress((p) => {
      setProgress((prev) =>
        prev.phase === "downloading"
          ? { phase: "downloading", progress: p }
          : prev,
      );
    }).then((fn) => {
      if (active) unlistenDownload = fn;
      else fn();
    });

    onFlashProgress((p) => {
      setProgress((prev) =>
        prev.phase === "flashing" ? { phase: "flashing", progress: p } : prev,
      );
    }).then((fn) => {
      if (active) unlistenFlash = fn;
      else fn();
    });

    return () => {
      active = false;
      unlistenDownload?.();
      unlistenFlash?.();
    };
  }, []);

  const flashMutation = useMutation<void, unknown, FlashVars>({
    mutationFn: async (vars) => {
      let imagePath: string;

      if (vars.sourceKind === "aircast") {
        if (!vars.release) throw new Error("No Aircast release available.");
        setProgress({ phase: "downloading", progress: null });
        const result = await downloadImage({
          downloadUrl: vars.release.image.download_url,
          checksumUrl: vars.release.image.checksum_url,
        });
        imagePath = result.image_path;
      } else {
        if (!vars.localPath) throw new Error("No local image selected.");
        imagePath = vars.localPath;
      }

      // Customization now happens inside the engine, so WiFi + hostname fold
      // into the single elevated flash — no separate provision step.
      setProgress({ phase: "flashing", progress: null });
      await flashImage({
        imagePath,
        targetDisk: vars.targetDisk,
        wifi: vars.wifi,
        hostname: vars.hostname,
        tailscale: vars.tailscale,
        access: vars.access,
        initFormat: "cloud-init" satisfies InitFormat,
      });
    },
    onSettled: () => {
      setProgress({ phase: "idle" });
    },
  });

  const localFileName = useMemo(() => {
    if (!localPath) return null;
    return localPath.split(/[\\/]/).pop() ?? localPath;
  }, [localPath]);

  const sourceReady =
    sourceKind === "aircast" ? release !== null : localPath !== null;
  const canProceed = sourceReady && selectedDisk !== "";

  async function handlePickLocal() {
    const path = await pickLocalImage();
    if (path) {
      setLocalPath(path);
      setSourceKind("local");
    }
  }

  function handleFlash() {
    if (!canProceed) return;

    setStep(STEP.write);

    const trimmedSsid = ssid.trim();
    // Country/regulatory domain is auto-detected on the backend; fall back to US.
    const detectedCountry = (wifiQuery.data?.country ?? "US").toUpperCase();
    const wifi: WifiConfig | null =
      trimmedSsid === ""
        ? null
        : {
            ssid: trimmedSsid,
            password,
            country: detectedCountry,
          };
    const trimmedHostname = hostname.trim();
    const hostnameValue = trimmedHostname === "" ? null : trimmedHostname;

    const trimmedKey = authKey.trim();
    const tailscale: TailscaleConfig | null =
      trimmedKey === ""
        ? null
        : { controlServer: controlServer.trim(), authKey: trimmedKey };

    const access = buildAccess(sshMode, sshKey, devicePassword);

    flashMutation.mutate({
      sourceKind,
      release,
      localPath,
      targetDisk: selectedDisk,
      wifi,
      hostname: hostnameValue,
      tailscale,
      access,
    });
  }

  async function handleCancel() {
    try {
      await cancelFlash();
    } catch {
      // ignore — surfaced via the rejected flash/download promise
    }
  }

  function flashAnother() {
    flashMutation.reset();
    setProgress({ phase: "idle" });
    setStep(STEP.os);
    void queryClient.invalidateQueries({ queryKey: ["devices"] });
  }

  // The write step (4) is reached only once a job has been started.
  const writing = flashMutation.isPending;

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
      <UpdateBanner />

      <div className="flex min-h-0 flex-1 overflow-hidden">
        <WizardSidebar
          current={step}
          highestReached={step}
          writing={writing}
          onSelect={(s) => setStep(s)}
        />

        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          {step === STEP.os ? (
            <StepImage
              sourceKind={sourceKind}
              onSourceKind={setSourceKind}
              release={release}
              releaseLoading={releasesQuery.isLoading}
              releaseError={releaseError}
              localFileName={localFileName}
              onPickLocal={handlePickLocal}
              canProceed={sourceReady}
              onNext={() => setStep(STEP.network)}
            />
          ) : step === STEP.network ? (
            <StepNetwork
              ssid={ssid}
              onSsid={setSsid}
              knownNetworks={wifiQuery.data?.known ?? []}
              scanning={wifiQuery.isFetching}
              onRescan={() => void wifiQuery.refetch()}
              password={password}
              onPassword={setPassword}
              showPassword={showPassword}
              onToggleShowPassword={() => setShowPassword((v) => !v)}
              hostname={hostname}
              onHostname={setHostname}
              controlServer={controlServer}
              onControlServer={setControlServer}
              authKey={authKey}
              onAuthKey={setAuthKey}
              sshMode={sshMode}
              onSshMode={setSshMode}
              sshKey={sshKey}
              onSshKey={setSshKey}
              detectedKeys={sshKeysQuery.data ?? []}
              onChooseKeyFile={handleChooseKeyFile}
              devicePassword={devicePassword}
              onDevicePassword={setDevicePassword}
              onBack={() => setStep(STEP.os)}
              onNext={() => setStep(STEP.storage)}
            />
          ) : step === STEP.storage ? (
            <StepStorage
              devices={devices}
              devicesLoading={devicesQuery.isLoading}
              selectedDisk={selectedDisk}
              onSelectDisk={setSelectedDisk}
              canProceed={canProceed}
              onBack={() => setStep(STEP.network)}
              onFlash={handleFlash}
            />
          ) : (
            <JobView
              success={flashMutation.isSuccess}
              error={
                flashMutation.isError
                  ? errorMessage(flashMutation.error, "Flashing failed.")
                  : null
              }
              progress={progress}
              onCancel={handleCancel}
              onReset={flashAnother}
            />
          )}
        </main>
      </div>
    </div>
  );
}

export default App;

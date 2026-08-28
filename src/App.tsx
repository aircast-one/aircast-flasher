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
  revealEventLog,
  track,
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
import { defaultHostname } from "@/components/default-hostname";
import { nextHostname } from "@/components/next-hostname";
import { randomHex } from "@/lib/random-id";
import { buildSummary } from "@/components/wizard-summary";
import { useSettings } from "@/lib/use-settings";
import { ConsentBanner } from "@/components/consent-banner";
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

const STEP_NAME: Record<WizardStep, string> = {
  [STEP.os]: "image",
  [STEP.network]: "network",
  [STEP.storage]: "storage",
  [STEP.write]: "write",
};

const DEFAULT_HOSTNAME = defaultHostname();

function pickDefaultRelease(releases: Release[]): Release | null {
  return releases.find((r) => !r.prerelease) ?? releases[0] ?? null;
}

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
  const [sourceKind, setSourceKind] = useState<SourceKind>("aircast");
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null);
  const [localPath, setLocalPath] = useState<string | null>(null);
  const [selectedDisk, setSelectedDisk] = useState<string>("");

  const { settings, update } = useSettings();
  const [showPassword, setShowPassword] = useState(false);
  const [authKey, setAuthKey] = useState("");
  const [sshMode, setSshMode] = useState<SshMode>("key-only");
  const [devicePassword, setDevicePassword] = useState("");

  const password = settings.wifiPassword;
  const setPassword = (wifiPassword: string) => update({ wifiPassword });
  const controlServer = settings.controlServer;
  const setControlServer = (controlServer: string) => update({ controlServer });
  const setSshKey = (authorizedKey: string) => update({ authorizedKey });
  const setSsid = (ssid: string) => update({ ssid });
  const setHostname = (hostname: string) => update({ hostname });

  const noWifi = settings.noWifi;
  const setNoWifi = (value: boolean) => update({ noWifi: value });

  const [step, setStep] = useState<WizardStep>(STEP.os);
  const [cancelled, setCancelled] = useState(false);
  const [lastVars, setLastVars] = useState<FlashVars | null>(null);

  const [progress, setProgress] = useState<FlashProgressState>({
    phase: "idle",
  });

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

  const ssid = settings.ssid ?? wifiQuery.data?.current ?? "";
  const hostname = settings.hostname ?? DEFAULT_HOSTNAME;

  const devices: BlockDevice[] = devicesQuery.data ?? [];
  const releases: Release[] = releasesQuery.data?.releases ?? [];
  const release = useMemo(() => {
    const chosen = releases.find((r) => r.version === selectedVersion);
    return chosen ?? pickDefaultRelease(releases);
  }, [releases, selectedVersion]);
  const releaseError = releasesQuery.isError
    ? errorMessage(releasesQuery.error, "Failed to load releases.")
    : releasesQuery.isSuccess && !release
      ? "No releases available."
      : null;
  useEffect(() => track("app_start"), []);

  // The wizard's own funnel: which step an operator reached, and where the
  // ones who never flash stop. Keyed on the step, so a step is counted once
  // however long they sit on it.
  useEffect(() => track("step_view", { step: STEP_NAME[step] }), [step]);

  // "No card detected" looks identical to "never tried" in a flash-only log.
  // Keyed on the count, so the 2s poll does not report the same card forever.
  useEffect(() => {
    if (onStorageStep) track("devices_listed", { count: devices.length });
  }, [onStorageStep, devices.length]);

  useEffect(() => {
    if (releaseError) track("releases_failed", { error: releaseError });
  }, [releaseError]);

  useEffect(() => {
    const list = devicesQuery.data ?? [];
    setSelectedDisk((prev) =>
      list.some((d) => d.path === prev)
        ? prev
        : list.length === 1
          ? list[0].path
          : "",
    );
  }, [devicesQuery.data]);
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
          jobId: vars.jobId,
          downloadUrl: vars.release.image.download_url,
          checksumUrl: vars.release.image.checksum_url,
        });
        imagePath = result.image_path;
      } else {
        if (!vars.localPath) throw new Error("No local image selected.");
        imagePath = vars.localPath;
      }
      setProgress({ phase: "flashing", progress: null });
      await flashImage({
        jobId: vars.jobId,
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
  // The card has to hold the *decompressed* image, which is several times the
  // download. Releases do not publish that size yet, so the backend hands us a
  // floor big enough for a real image (and the exact check still runs against
  // the decompressed file before anything is written).
  const requiredCardBytes =
    sourceKind === "aircast" && release !== null
      ? (release.image.uncompressed_size ?? releasesQuery.data?.min_card_bytes)
      : undefined;
  const selectedDevice = devices.find((d) => d.path === selectedDisk) ?? null;
  const cardTooSmall =
    requiredCardBytes !== undefined &&
    selectedDevice !== null &&
    selectedDevice.size > 0 &&
    selectedDevice.size < requiredCardBytes;
  const canProceed = sourceReady && selectedDisk !== "" && !cardTooSmall;

  const remoteEnrolled = authKey.trim() !== "";
  const detectedKeys = sshKeysQuery.data ?? [];
  // Prefill the one detected key, but never fight an operator who cleared it:
  // null is "never set", "" is "cleared on purpose". Derived at render, so
  // there is no effect and no state to keep in sync.
  const sshKey =
    settings.authorizedKey ??
    (detectedKeys.length === 1 ? detectedKeys[0].contents : "");

  const imageLabel =
    sourceKind === "aircast"
      ? release
        ? `Aircast OS ${release.version}`
        : "No release"
      : (localFileName ?? "No file");

  const summary = buildSummary({
    image: imageLabel,
    hostname,
    ssid,
    noWifi,
    authKey,
    controlServer,
    sshMode,
    sshKey,
    detectedKeys,
    devicePassword,
  });

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
    setCancelled(false);

    const trimmedSsid = ssid.trim();
    const detectedCountry = (wifiQuery.data?.country ?? "US").toUpperCase();
    const wifi: WifiConfig | null =
      noWifi || trimmedSsid === ""
        ? null
        : {
            ssid: trimmedSsid,
            password,
            country: detectedCountry,
          };
    const trimmedHostname = hostname.trim();

    const trimmedKey = authKey.trim();
    const tailscale: TailscaleConfig | null =
      trimmedKey === ""
        ? null
        : { controlServer: controlServer.trim(), authKey: trimmedKey };

    const access = buildAccess(sshMode, sshKey, devicePassword);

    const vars: FlashVars = {
      jobId: randomHex(8),
      sourceKind,
      release,
      localPath,
      targetDisk: selectedDisk,
      wifi,
      hostname: trimmedHostname,
      tailscale,
      access,
    };
    setLastVars(vars);
    flashMutation.mutate(vars);
  }

  function answerTelemetry(share: boolean) {
    update({ telemetry: share });
    // Opting in is itself the first event, and the only one that can confirm a
    // build's sink works at all.
    if (share) track("consent", { shared: true });
  }

  function handleCancel() {
    setCancelled(true);
    void cancelFlash().catch(() => undefined);
  }

  function retryFlash(vars: FlashVars) {
    setCancelled(false);
    flashMutation.mutate({ ...vars, jobId: randomHex(8) });
  }

  function startOver() {
    flashMutation.reset();
    setProgress({ phase: "idle" });
    setCancelled(false);
    setStep(STEP.os);
    void queryClient.invalidateQueries({ queryKey: ["devices"] });
  }

  function flashAnother() {
    setHostname(nextHostname(hostname));
    startOver();
  }
  const writing = flashMutation.isPending;

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
      <ConsentBanner
        asked={settings.telemetry !== null}
        onAnswer={answerTelemetry}
      />
      <UpdateBanner suspended={writing} />

      <div className="flex min-h-0 flex-1 overflow-hidden">
        <WizardSidebar
          current={step}
          highestReached={step}
          writing={writing}
          onSelect={(s) => setStep(s)}
          telemetry={settings.telemetry ?? false}
          onTelemetry={answerTelemetry}
        />

        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          {step === STEP.os ? (
            <StepImage
              sourceKind={sourceKind}
              onSourceKind={setSourceKind}
              release={release}
              releases={releases}
              onSelectVersion={setSelectedVersion}
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
              noWifi={noWifi}
              onNoWifi={setNoWifi}
              remote={{
                controlServer,
                onControlServer: setControlServer,
                authKey,
                onAuthKey: setAuthKey,
              }}
              access={{
                sshMode,
                onSshMode: setSshMode,
                sshKey,
                onSshKey: setSshKey,
                detectedKeys,
                onChooseKeyFile: handleChooseKeyFile,
                devicePassword,
                onDevicePassword: setDevicePassword,
              }}
              onBack={() => setStep(STEP.os)}
              onNext={() => setStep(STEP.storage)}
            />
          ) : step === STEP.storage ? (
            <StepStorage
              devices={devices}
              devicesLoading={devicesQuery.isLoading}
              selectedDisk={selectedDisk}
              onSelectDisk={setSelectedDisk}
              requiredCardBytes={requiredCardBytes}
              summary={summary}
              canProceed={canProceed}
              onBack={() => setStep(STEP.network)}
              onEditStep={setStep}
              onFlash={handleFlash}
            />
          ) : (
            <JobView
              hostname={hostname.trim()}
              remoteEnrolled={remoteEnrolled}
              success={flashMutation.isSuccess}
              cancelled={cancelled && flashMutation.isError}
              error={
                flashMutation.isError
                  ? errorMessage(flashMutation.error, "Flashing failed.")
                  : null
              }
              progress={progress}
              onCancel={handleCancel}
              onRetry={lastVars === null ? null : () => retryFlash(lastVars)}
              onStartOver={startOver}
              onReset={flashAnother}
              onRevealLog={() => void revealEventLog()}
            />
          )}
        </main>
      </div>
    </div>
  );
}

export default App;

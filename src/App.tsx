import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  cancelFlash,
  cloudCreateKeyset,
  detectSshKeys,
  downloadImage,
  fetchTangAdv,
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
  EncryptionConfig,
  TangServer,
  BlockDevice,
  DownloadProgress,
  InitFormat,
  Release,
  SshMode,
  TailscaleConfig,
  WifiConfig,
} from "@/types";
import { errorMessage } from "@/lib/format";
import { defaultHostname } from "@/components/default-hostname";
import { nextFreeHostname } from "@/components/next-hostname";
import { randomHex } from "@/lib/random-id";
import { buildSummary } from "@/components/wizard-summary";
import { isCaptured } from "@/components/encryption";
import { cardLabel, withCloudServer } from "@/components/cloud";
import { isValidControlServer } from "@/components/step-network.validation";
import { useSettings } from "@/lib/use-settings";
import { useLocalTailscale } from "@/lib/use-local-tailscale";
import { ConsentBanner } from "@/components/consent-banner";
import { UpdateBanner } from "@/components/update-banner";
import { WizardSidebar } from "@/components/wizard-sidebar";
import { StepStorage } from "@/components/step-storage";
import { StepImage } from "@/components/step-image";
import { StepNetwork } from "@/components/step-network";
import { StepAccess } from "@/components/step-access";
import { JobView } from "@/components/job-view";
import { DevicesView } from "@/components/devices-view";
import { CloudCardsView } from "@/components/cloud-cards-view";
import {
  STEP,
  type FlashProgressState,
  type FleetView,
  type FlashVars,
  type SourceKind,
  type WizardStep,
} from "@/components/wizard-types";

const DEVICE_POLL_INTERVAL_MS = 2000;

const STEP_NAME: Record<WizardStep, string> = {
  [STEP.os]: "image",
  [STEP.network]: "network",
  [STEP.access]: "access",
  [STEP.storage]: "storage",
  [STEP.write]: "write",
};

const FLASHED_HISTORY_LIMIT = 200;

function useTakenHostnames(flashed: readonly string[]): string[] {
  const local = useLocalTailscale();
  const peers = (local?.peers ?? []).map((peer) => peer.hostName);
  return useMemo(
    () => [...peers, ...flashed],
    [peers.join("\u0000"), flashed.join("\u0000")],
  );
}

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
  const [pickedDisk, setPickedDisk] = useState<string>("");

  const [fightMode, setFightMode] = useState(false);
  const { settings, update, forget } = useSettings(fightMode);
  const [showPassword, setShowPassword] = useState(false);
  const [authKey, setAuthKey] = useState("");
  const [sshMode, setSshMode] = useState<SshMode>("key-only");
  const [devicePassword, setDevicePassword] = useState("");

  const [encryptEnabled, setEncryptEnabled] = useState(false);
  const [tangServers, setTangServers] = useState<TangServer[]>([
    { url: "", adv: "" },
  ]);
  const [threshold, setThreshold] = useState(1);
  const [cloudKeyServer, setCloudKeyServer] = useState(false);
  const [recoveryKey, setRecoveryKey] = useState("");
  const [recoverySaved, setRecoverySaved] = useState(false);
  const enableEncryption = (on: boolean) => {
    setEncryptEnabled(on);
    if (!on) {
      setRecoverySaved(false);
      return;
    }
    if (recoveryKey === "") setRecoveryKey(randomHex(24));
    if (settings.tangCloud) setCloudKeyServer(true);
    const remembered = settings.tangUrls;
    if (remembered.length > 0 && tangServers.every((s) => s.url === "")) {
      setThreshold(Math.max(1, Math.min(settings.tangThreshold || 1, remembered.length)));
      setTangServers(remembered.map((url) => ({ url, adv: "" })));
      Promise.all(
        remembered.map(async (url) => {
          try {
            return { url, adv: await fetchTangAdv(url) };
          } catch {
            return { url, adv: "" };
          }
        }),
      ).then(setTangServers);
    }
  };

  const enableFightMode = (on: boolean) => {
    setFightMode(on);
    if (!on) return;
    forget();
    if (sshMode === "password") setSshMode("key-only");
  };

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
  const [fleetView, setFleetView] = useState<FleetView | null>(null);
  const [cancelled, setCancelled] = useState(false);
  const [lastVars, setLastVars] = useState<FlashVars | null>(null);

  const [progress, setProgress] = useState<FlashProgressState>({
    phase: "idle",
  });
  const [downloadProgress, setDownloadProgress] =
    useState<DownloadProgress | null>(null);

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
  const takenHostnames = useTakenHostnames(settings.flashedHostnames);
  const suggested = defaultHostname(takenHostnames);
  const hostname = settings.hostname ?? suggested;

  const devices: BlockDevice[] = devicesQuery.data ?? [];
  const selectedDisk = devices.some((d) => d.path === pickedDisk)
    ? pickedDisk
    : devices.length === 1
      ? devices[0].path
      : "";
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

  useEffect(() => track("step_view", { step: STEP_NAME[step] }), [step]);

  useEffect(() => {
    if (onStorageStep) track("devices_listed", { count: devices.length });
  }, [onStorageStep, devices.length]);

  useEffect(() => {
    if (releaseError) track("releases_failed", { error: releaseError });
  }, [releaseError]);

  useEffect(() => {
    let active = true;
    let unlistenDownload: (() => void) | undefined;
    let unlistenFlash: (() => void) | undefined;

    onDownloadProgress((p) => {
      setDownloadProgress(p);
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

  const chosen = selectedVersion !== null || step > STEP.os;
  const imageUrl =
    sourceKind === "aircast" && chosen
      ? (release?.image.download_url ?? null)
      : null;

  const imageDownload = useQuery({
    queryKey: ["image", imageUrl],
    queryFn: () =>
      downloadImage({
        jobId: randomHex(8),
        downloadUrl: release!.image.download_url,
        checksumUrl: release!.image.checksum_url,
      }),
    enabled: imageUrl !== null,
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
  });

  const cacheStatus =
    imageUrl === null
      ? null
      : imageDownload.isSuccess
        ? imageDownload.data.cached
          ? "Already on this computer — nothing to download."
          : "Downloaded — ready to write."
        : imageDownload.isError
          ? "Download failed — it retries when you write the card."
          : downloadProgress && downloadProgress.total_bytes > 0
            ? `Downloading in the background… ${Math.min(100, Math.round(downloadProgress.percent))}%`
            : "Downloading in the background…";

  const flashMutation = useMutation<EncryptionConfig | null, unknown, FlashVars>({
    mutationFn: async (vars) => {
      let imagePath: string;

      if (vars.sourceKind === "aircast") {
        if (!vars.release) throw new Error("No Aircast release available.");
        setProgress({ phase: "downloading", progress: null });
        const result = await queryClient.fetchQuery({
          queryKey: ["image", vars.release.image.download_url],
          queryFn: () =>
            downloadImage({
              jobId: vars.jobId,
              downloadUrl: vars.release!.image.download_url,
              checksumUrl: vars.release!.image.checksum_url,
            }),
          staleTime: Infinity,
          gcTime: Infinity,
          retry: false,
        });
        imagePath = result.image_path;
      } else {
        if (!vars.localPath) throw new Error("No local image selected.");
        imagePath = vars.localPath;
      }
      const encryption =
        vars.encryption !== null && vars.cloudKeyServer
          ? withCloudServer(
              vars.encryption,
              await cloudCreateKeyset(cardLabel(vars.hostname)),
            )
          : vars.encryption;
      setProgress({ phase: "flashing", progress: null });
      await flashImage({
        jobId: vars.jobId,
        imagePath,
        targetDisk: vars.targetDisk,
        wifi: vars.wifi,
        hostname: vars.hostname,
        tailscale: vars.tailscale,
        access: vars.access,
        encryption,
        fightMode: vars.fightMode,
        initFormat: "cloud-init" satisfies InitFormat,
      });
      return encryption;
    },
    onSuccess: (_result, vars) => {
      if (vars.fightMode) return;
      if (vars.encryption) {
        update({
          tangUrls: vars.encryption.tangServers.map((s) => s.url),
          tangThreshold: vars.encryption.threshold,
          tangCloud: vars.cloudKeyServer,
        });
      }
      const written = vars.hostname.trim();
      if (written === "" || settings.flashedHostnames.includes(written)) return;
      update({
        flashedHostnames: [...settings.flashedHostnames, written].slice(
          -FLASHED_HISTORY_LIMIT,
        ),
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
  const requiredCardBytes =
    sourceKind === "aircast" && release !== null
      ? (release.image.uncompressed_size ?? releasesQuery.data?.min_card_bytes)
      : undefined;
  const selectedDevice = devices.find((d) => d.path === selectedDisk);
  const cardTooSmall =
    requiredCardBytes !== undefined &&
    selectedDevice !== undefined &&
    selectedDevice.size > 0 &&
    selectedDevice.size < requiredCardBytes;
  const canProceed = sourceReady && selectedDisk !== "" && !cardTooSmall;

  const remoteEnrolled = authKey.trim() !== "";
  const detectedKeys = sshKeysQuery.data ?? [];
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
    encryption: encryptEnabled
      ? { tang: tangServers.some(isCaptured), cloud: cloudKeyServer }
      : null,
    fightMode,
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
    const trimmedControl = controlServer.trim();
    const tailscale: TailscaleConfig | null =
      trimmedKey === "" || !isValidControlServer(trimmedControl)
        ? null
        : { controlServer: trimmedControl, authKey: trimmedKey };

    const access = buildAccess(sshMode, sshKey, devicePassword);

    const capturedServers = tangServers
      .filter(isCaptured)
      .map((s) => ({ url: s.url.trim(), adv: s.adv }));
    const keyServers = capturedServers.length + (cloudKeyServer ? 1 : 0);
    const encryption: EncryptionConfig | null =
      encryptEnabled &&
      keyServers >= 1 &&
      threshold >= 1 &&
      threshold <= keyServers
        ? { tangServers: capturedServers, threshold, recoveryKey }
        : null;

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
      encryption,
      cloudKeyServer: encryptEnabled && cloudKeyServer,
      fightMode,
    };
    setLastVars(vars);
    flashMutation.mutate(vars);
  }

  function answerTelemetry(share: boolean) {
    update({ telemetry: share });
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
    setHostname(nextFreeHostname(hostname, takenHostnames));
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
          onSelect={(s) => {
            setFleetView(null);
            setStep(s);
          }}
          fleetView={fleetView}
          onFleetView={(view) =>
            setFleetView((current) => (current === view ? null : view))
          }
          telemetry={settings.telemetry ?? false}
          onTelemetry={answerTelemetry}
        />

        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          {fleetView === "devices" ? (
            <DevicesView
              onFlash={() => {
                setFleetView(null);
                setStep(STEP.os);
              }}
            />
          ) : fleetView === "cloud" ? (
            <CloudCardsView />
          ) : step === STEP.os ? (
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
              cacheStatus={cacheStatus}
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
              onBack={() => setStep(STEP.os)}
              onNext={() => setStep(STEP.access)}
            />
          ) : step === STEP.access ? (
            <StepAccess
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
              encryption={{
                enabled: encryptEnabled,
                onEnabled: enableEncryption,
                servers: tangServers,
                onServers: setTangServers,
                threshold,
                onThreshold: setThreshold,
                cloud: cloudKeyServer,
                onCloud: setCloudKeyServer,
                recoveryKey,
                recoverySaved,
                onRecoverySaved: setRecoverySaved,
                wifiUplink: !noWifi && ssid.trim() !== "",
                hostname,
              }}
              fight={{ enabled: fightMode, onEnabled: enableFightMode }}
              onBack={() => setStep(STEP.network)}
              onNext={() => setStep(STEP.storage)}
            />
          ) : step === STEP.storage ? (
            <StepStorage
              devices={devices}
              devicesLoading={devicesQuery.isLoading}
              selectedDisk={selectedDisk}
              onSelectDisk={setPickedDisk}
              requiredCardBytes={requiredCardBytes}
              summary={summary}
              canProceed={canProceed}
              onBack={() => setStep(STEP.access)}
              onEditStep={setStep}
              onFlash={handleFlash}
            />
          ) : (
            <JobView
              hostname={hostname.trim()}
              remoteEnrolled={remoteEnrolled}
              encryption={flashMutation.data ?? null}
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

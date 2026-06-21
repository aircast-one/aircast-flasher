import { useEffect, useMemo, useState } from "react";
import {
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import {
  cancelFlash,
  downloadImage,
  flashImage,
  listBlockDevices,
  listReleases,
  listWifiNetworks,
  onDownloadProgress,
  onFlashProgress,
  pickLocalImage,
} from "@/api";
import type { BlockDevice, InitFormat, Release, WifiConfig } from "@/types";
import { errorMessage } from "@/lib/format";
import { useLocalStorage } from "@/lib/use-local-storage";
import { UpdateBanner } from "@/components/update-banner";
import { WizardSidebar } from "@/components/wizard-sidebar";
import { StepStorage } from "@/components/step-storage";
import { StepImage } from "@/components/step-image";
import { StepNetwork } from "@/components/step-network";
import { JobView } from "@/components/job-view";
import type {
  FlashProgressState,
  FlashVars,
  SourceKind,
  WizardStep,
} from "@/components/wizard-types";

function pickDefaultRelease(releases: Release[]): Release | null {
  return releases.find((r) => !r.prerelease) ?? releases[0] ?? null;
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

  // Wizard
  const [step, setStep] = useState<WizardStep>(1);

  // Live progress from Tauri events.
  const [progress, setProgress] = useState<FlashProgressState>({
    phase: "idle",
  });

  const devicesQuery = useQuery({
    queryKey: ["devices"],
    queryFn: listBlockDevices,
  });

  const releasesQuery = useQuery({
    queryKey: ["releases"],
    queryFn: () => listReleases(),
  });

  const wifiQuery = useQuery({
    queryKey: ["wifi"],
    queryFn: listWifiNetworks,
  });

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

  // Keep the selected disk valid as the device list changes.
  useEffect(() => {
    setSelectedDisk((prev) =>
      devices.some((d) => d.path === prev) ? prev : (devices[0]?.path ?? ""),
    );
  }, [devices]);

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

    setStep(4);

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

    flashMutation.mutate({
      sourceKind,
      release,
      localPath,
      targetDisk: selectedDisk,
      wifi,
      hostname: hostnameValue,
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
    setStep(1);
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
        {step === 1 ? (
          <StepStorage
            devices={devices}
            devicesLoading={devicesQuery.isFetching}
            selectedDisk={selectedDisk}
            onSelectDisk={setSelectedDisk}
            onRefresh={() => void devicesQuery.refetch()}
            canProceed={selectedDisk !== ""}
            onNext={() => setStep(2)}
          />
        ) : step === 2 ? (
          <StepImage
            sourceKind={sourceKind}
            onSourceKind={setSourceKind}
            release={release}
            releaseLoading={releasesQuery.isLoading}
            releaseError={releaseError}
            localFileName={localFileName}
            onPickLocal={handlePickLocal}
            canProceed={sourceReady}
            onBack={() => setStep(1)}
            onNext={() => setStep(3)}
          />
        ) : step === 3 ? (
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
            onBack={() => setStep(2)}
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

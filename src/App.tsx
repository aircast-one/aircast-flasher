import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  cancelFlash,
  cloudCheckSignIn,
  cloudCreateKeyset,
  cloudProvisionDevice,
  mintAuthKey,
  detectSshKeys,
  downloadImage,
  prepareAircastd,
  prepareImage,
  fetchTangAdv,
  flashImage,
  listenDebugRequests,
  listBlockDevices,
  listBoards,
  listReleases,
  listRunningBoards,
  listWifiNetworks,
  onBackupsChanged,
  onDownloadProgress,
  onFlashProgress,
  pickAndReadPublicKey,
  setBackgroundBackups,
  pickLocalImage,
  revealEventLog,
  track,
} from "@/api";
import type {
  AccessConfig,
  AircastdOutcome,
  Board,
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
import { fitsBoard, isUsbTarget, usbFamily } from "@/components/usb-target";
import { randomHex } from "@/lib/random-id";
import { buildSummary } from "@/components/wizard-summary";
import {
  encryptionProblems,
  hasUrl,
  isCaptured,
  releaseSupportsEncryption,
  releaseSupportsFight,
} from "@/components/encryption";
import {
  cardLabel,
  linkedAccount,
  withCloudServer,
  type CloudLink,
  type CloudOutcome,
} from "@/components/cloud";
import {
  MIN_FIGHT_PASSWORD,
  fightModeUnreachable,
  mintsKeys,
  needsHeadscaleUser,
  defaultSshMode,
  isWeakDevicePassword,
  joinsTailnet,
  networkChoices,
} from "@/components/step-network.validation";
import { useSettings } from "@/lib/use-settings";
import { useLocalTailscale } from "@/lib/use-local-tailscale";
import { BACKUPS_KEY, useBackups } from "@/lib/use-backups";
import {
  backupMarks,
  cardPreset,
  rejoinsTailnet,
  restoreProblems,
} from "@/components/restore-card";
import { answerConsent, asksConsent } from "@/components/consent";
import {
  runDebugRequest,
  type DebugControls,
  type DebugPage,
  type TextField,
  type ToggleField,
} from "@/components/debug-requests";
import { UpdateBanner } from "@/components/update-banner";
import { WizardSidebar } from "@/components/wizard-sidebar";
import { StepStorage } from "@/components/step-storage";
import type { Problem } from "@/components/step-shell";
import { StepConsent } from "@/components/step-consent";
import { StepImage } from "@/components/step-image";
import { StepNetwork } from "@/components/step-network";
import { StepAccess } from "@/components/step-access";
import { JobView } from "@/components/job-view";
import { DevicesView } from "@/components/devices-view";
import {
  ACCOUNT_KEY,
  CloudCardsView,
  useCloudAccount,
} from "@/components/cloud-cards-view";
import {
  STEP,
  type FlashProgressState,
  type FleetView,
  type FlashVars,
  type RestoreDraft,
  type SourceKind,
  type WizardStep,
} from "@/components/wizard-types";

const DEVICE_POLL_INTERVAL_MS = 2000;
const RELEASES_POLL_INTERVAL_MS = 60_000;

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
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null);
  const [pickedDisk, setPickedDisk] = useState<string>("");

  const { settings, update, loaded } = useSettings();
  const sourceKind: SourceKind = settings.useLocalImage ? "local" : "aircast";
  const setSourceKind = (kind: SourceKind) =>
    update({ useLocalImage: kind === "local" });
  const setBoard = (next: Board) => {
    if (next.id === settings.board) return;
    setSelectedVersion(null);
    update({ board: next.id, noWifi: !next.builtinWifi });
  };
  const localPath = settings.localImagePath;
  const fightMode = settings.fightMode;
  const [showPassword, setShowPassword] = useState(false);
  const authKey = settings.authKey;
  const setAuthKey = (authKey: string) =>
    update({ authKey, authKeyWrittenTo: null });
  const setSshMode = (sshMode: SshMode) => update({ sshMode });
  const devicePassword = settings.devicePassword;
  const setDevicePassword = (devicePassword: string) =>
    update({ devicePassword });
  const backups = useBackups();
  const [restore, setRestore] = useState<RestoreDraft | null>(null);
  const restoreBackup = backups.find((b) => b.id === restore?.backup) ?? null;
  const restoreCard = restoreBackup?.card ?? null;
  const rejoins = rejoinsTailnet(restoreCard, restore?.sameDrone ?? null);
  const remoteAccess = settings.remoteAccess ?? true;
  const tailnetKey = remoteAccess && !rejoins ? authKey : "";
  const tailnetServer = settings.selfHosted ? settings.controlServer : "";
  const headscaleUser = settings.headscaleUser;

  const encryptEnabled = settings.encryption;
  const [tangServers, setTangServerRows] = useState<TangServer[]>([
    { url: "", adv: "" },
  ]);
  const setTangServers = (servers: TangServer[]) => {
    setTangServerRows(servers);
    update({ tangUrls: servers.map((s) => s.url.trim()).filter((u) => u !== "") });
  };
  const threshold = Math.max(1, settings.tangThreshold);
  const setThreshold = (tangThreshold: number) => update({ tangThreshold });
  const cloudKeyServer = settings.tangCloud;
  const setCloudKeyServer = (tangCloud: boolean) => update({ tangCloud });
  const cloudData = useCloudAccount().data;
  const cloudAccount = typeof cloudData === "string" ? cloudData : null;
  const cloudSignedIn = cloudAccount !== null;
  const cloudOn = cloudKeyServer && cloudSignedIn;
  const [recoveryKey] = useState(() => randomHex(24));
  const [recoverySaved, setRecoverySaved] = useState(false);
  const loadTangServers = (urls: string[]) => {
    if (urls.length === 0) {
      setTangServerRows([{ url: "", adv: "" }]);
      return;
    }
    setTangServerRows(urls.map((url) => ({ url, adv: "" })));
    Promise.all(
      urls.map(async (url) => {
        try {
          return { url, adv: await fetchTangAdv(url) };
        } catch {
          return { url, adv: "" };
        }
      }),
    ).then(setTangServerRows);
  };
  const restoreTangServers = () => {
    const remembered = settings.tangUrls;
    if (remembered.length === 0 || tangServers.some(hasUrl)) return;
    loadTangServers(remembered);
  };
  const enableEncryption = (on: boolean) => {
    update({ encryption: on });
    if (!on) {
      setRecoverySaved(false);
      return;
    }
    restoreTangServers();
  };
  useEffect(() => {
    if (loaded && settings.encryption) restoreTangServers();
  }, [loaded]);

  const enableFightMode = (on: boolean) =>
    update({
      fightMode: on,
      ...(on ? { remoteAccess: true } : {}),
    });

  const password = settings.wifiPassword;
  const setPassword = (wifiPassword: string) => update({ wifiPassword });
  const controlServer = settings.controlServer;
  const setControlServer = (controlServer: string) => update({ controlServer });
  const setSshKey = (authorizedKey: string) => update({ authorizedKey });
  const setSsid = (ssid: string) => update({ ssid });
  const setHostname = (hostname: string) => update({ hostname });

  const noWifi = settings.noWifi;
  const setNoWifi = (value: boolean) => update({ noWifi: value });

  const chooseRestore = (next: RestoreDraft | null) => {
    setRestore(next);
    const source = backups.find((b) => b.id === next?.backup);
    const card = next?.backup === restore?.backup ? null : (source?.card ?? null);
    update({
      ...(next?.sameDrone && source ? { hostname: source.hostname } : {}),
      ...(card === null ? {} : cardPreset(card)),
    });
    if (card?.encryption && card.encryption.threshold > 0) {
      loadTangServers(card.encryption.tangUrls);
    }
  };

  const [step, setStep] = useState<WizardStep>(STEP.os);
  const [fleetView, setFleetView] = useState<FleetView | null>(null);
  const [cancelled, setCancelled] = useState(false);
  const [lastVars, setLastVars] = useState<FlashVars | null>(null);
  const [cardTouched, setCardTouched] = useState(false);

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
  const runningBoardsQuery = useQuery({
    queryKey: ["running-boards"],
    queryFn: listRunningBoards,
    enabled: onStorageStep,
    refetchInterval: onStorageStep ? DEVICE_POLL_INTERVAL_MS : false,
  });

  const onOsStep = step === STEP.os;
  const boardsQuery = useQuery({
    queryKey: ["boards"],
    queryFn: listBoards,
    staleTime: Infinity,
  });
  const boards = boardsQuery.data ?? [];
  const board = boards.find((b) => b.id === settings.board) ?? null;
  const releasesQuery = useQuery({
    queryKey: ["releases", settings.board],
    queryFn: () => listReleases(settings.board),
    enabled: settings.board !== null,
    refetchInterval: onOsStep ? RELEASES_POLL_INTERVAL_MS : false,
    refetchOnWindowFocus: onOsStep,
  });

  const wifiQuery = useQuery({
    queryKey: ["wifi"],
    queryFn: listWifiNetworks,
    refetchOnWindowFocus: true,
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
  const targets = devices.filter((d) => fitsBoard(d, board));
  const selectedDisk = targets.some((d) => d.path === pickedDisk)
    ? pickedDisk
    : targets.length === 1
      ? targets[0].path
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

  useEffect(() => {
    const unlisten = onBackupsChanged(
      () => void queryClient.invalidateQueries({ queryKey: BACKUPS_KEY }),
    );
    return () => void unlisten.then((fn) => fn());
  }, []);

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

  const downloadedPath = imageDownload.data?.image_path ?? null;
  const imageUnpack = useQuery({
    queryKey: ["unpack", downloadedPath],
    queryFn: () => prepareImage(downloadedPath!).then(() => true),
    enabled: downloadedPath !== null,
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
  });

  const cacheStatus =
    imageUrl === null
      ? null
      : imageDownload.isSuccess
        ? imageUnpack.isFetching
          ? "Downloaded — unpacking in the background…"
          : imageDownload.data.cached
            ? "Already on this computer — nothing to download."
            : "Downloaded — ready to write."
        : imageDownload.isError
          ? "Download failed — it retries when you write the card."
          : downloadProgress && downloadProgress.total_bytes > 0
            ? `Downloading in the background… ${Math.min(100, Math.round(downloadProgress.percent))}%`
            : "Downloading in the background…";

  const flashMutation = useMutation<
    {
      encryption: EncryptionConfig | null;
      cloudCard: string | null;
      cloud: CloudOutcome;
      aircastd: AircastdOutcome;
    },
    unknown,
    FlashVars
  >({
    mutationFn: async (vars) => {
      setCardTouched(false);
      const cloud = vars.encryption !== null && vars.cloudKeyServer;
      if (cloud) await cloudCheckSignIn();
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
      const keyset =
        vars.encryption !== null && cloud
          ? await cloudCreateKeyset(cardLabel(vars.hostname))
          : null;
      const encryption =
        vars.encryption !== null && keyset !== null
          ? withCloudServer(vars.encryption, keyset)
          : vars.encryption;
      const tailscale =
        vars.tailscale !== null && mintsKeys(vars.tailscale.authKey)
          ? {
              ...vars.tailscale,
              authKey: await mintAuthKey(
                vars.tailscale.controlServer,
                vars.tailscale.authKey,
                vars.headscaleUser,
              ),
            }
          : vars.tailscale;
      const account = vars.cloudAccount;
      const link =
        account === null
          ? null
          : await cloudProvisionDevice(cardLabel(vars.hostname)).then(
              (enroll) => ({ enroll, cloud: { account } }),
              (e: unknown) => ({
                enroll: null,
                cloud: { error: errorMessage(e, "Aircast Cloud did not answer.") },
              }),
            );
      setProgress({ phase: "flashing", progress: null });
      setCardTouched(true);
      const aircastd = await flashImage({
        jobId: vars.jobId,
        imagePath,
        targetDisk: vars.targetDisk,
        wifi: vars.wifi,
        hostname: vars.hostname,
        tailscale,
        access: vars.access,
        encryption,
        fightMode: vars.fightMode,
        restore: vars.restore,
        cloudEnroll: link?.enroll ?? null,
        initFormat: "cloud-init" satisfies InitFormat,
      });
      return {
        encryption,
        cloudCard: keyset?.id ?? null,
        cloud: link?.cloud ?? null,
        aircastd,
      };
    },
    onSuccess: (result, vars) => {
      if (result.cloud !== null && "error" in result.cloud)
        void queryClient.invalidateQueries({ queryKey: ACCOUNT_KEY });
      const written = vars.hostname.trim();
      const remembered =
        written === "" || settings.flashedHostnames.includes(written);
      update({
        ...(vars.tailscale === null || mintsKeys(vars.tailscale.authKey)
          ? {}
          : {
              authKeyWrittenTo: {
                hostname: written,
                at: new Date().toISOString(),
              },
            }),
        ...(remembered
          ? {}
          : {
              flashedHostnames: [...settings.flashedHostnames, written].slice(
                -FLASHED_HISTORY_LIMIT,
              ),
            }),
      });
    },
    onError: () => queryClient.invalidateQueries({ queryKey: ACCOUNT_KEY }),
    onSettled: () => {
      setProgress({ phase: "idle" });
    },
  });

  const localFileName = useMemo(() => {
    if (!localPath) return null;
    return localPath.split(/[\\/]/).pop() ?? localPath;
  }, [localPath]);

  const aircastdImage =
    sourceKind === "aircast"
      ? chosen
        ? (release?.version ?? null)
        : null
      : localFileName;
  const aircastdQuery = useQuery({
    queryKey: ["aircastd", aircastdImage],
    queryFn: () => prepareAircastd(aircastdImage!),
    enabled: aircastdImage !== null,
    retry: false,
  });

  const sourceReady =
    sourceKind === "aircast" ? release !== null : localPath !== null;
  const encryptionAvailable =
    sourceKind === "local" ||
    (release !== null && releaseSupportsEncryption(release.version));
  const encrypting = encryptEnabled && encryptionAvailable;
  const fightAvailable =
    sourceKind === "local" ||
    (release !== null && releaseSupportsFight(release.version));
  const fighting = fightMode && fightAvailable;
  const requiredCardBytes =
    sourceKind === "aircast" && release !== null
      ? (release.image.uncompressed_size ?? releasesQuery.data?.min_card_bytes)
      : undefined;
  const detectedKeys = sshKeysQuery.data ?? [];
  const sshKey =
    settings.authorizedKey ??
    (detectedKeys.length === 1 ? detectedKeys[0].contents : "");
  const sshMode: SshMode = settings.sshMode ?? defaultSshMode(sshKey);
  const fightLoginMissing =
    fighting &&
    ((sshMode === "key-only" && sshKey.trim() === "") ||
      (sshMode === "password" && isWeakDevicePassword(devicePassword, true)));
  const selectedDevice = devices.find((d) => d.path === selectedDisk);
  const cloudLink: CloudLink = {
    account: cloudAccount,
    on: settings.cloudLink ?? true,
    sameDrone: restore?.sameDrone === true,
    usb: isUsbTarget(selectedDisk),
  };
  const cardTooSmall =
    requiredCardBytes !== undefined &&
    selectedDevice !== undefined &&
    selectedDevice.size > 0 &&
    selectedDevice.size < requiredCardBytes;
  const storageProblems: Problem[] = [
    ...(sourceReady
      ? []
      : [
          {
            message: "Pick the operating system to write",
            fix: () => setStep(STEP.os),
          },
        ]),
    ...(selectedDisk === ""
      ? [
          {
            message:
              targets.length > 0
                ? "Pick where to write"
                : usbFamily(board) !== null
                  ? "Insert an SD card or connect the board in flashing mode"
                  : "Insert an SD card",
          },
        ]
      : []),
    ...(cardTooSmall
      ? [{ message: "This card is too small for the image — insert a bigger one" }]
      : []),
    ...(!rejoins && fightModeUnreachable(fighting, tailnetKey, tailnetServer)
      ? [
          {
            message:
              "Fight mode needs Remote access with a pre-auth key — the tailnet is the only way in",
            fix: () => setStep(STEP.access),
          },
        ]
      : []),
    ...(needsHeadscaleUser(settings.selfHosted, tailnetKey, headscaleUser)
      ? [
          {
            message: "Pick the Headscale user the drone joins as",
            fix: () => setStep(STEP.access),
          },
        ]
      : []),
    ...(fightLoginMissing
      ? [
          {
            message: `Fight mode needs an SSH key, a password of ${MIN_FIGHT_PASSWORD}+ characters, or SSH turned off — the image's default login would stay on`,
            fix: () => setStep(STEP.access),
          },
        ]
      : []),
    ...encryptionProblems({
      enabled: encrypting,
      servers: tangServers,
      threshold,
      cloud: cloudOn,
      recoverySaved,
    }).map(({ message }) => ({ message, fix: () => setStep(STEP.access) })),
    ...restoreProblems(restore, backups).map(({ message }) => ({
      message,
      fix: () => setStep(STEP.network),
    })),
  ];
  const canProceed = storageProblems.length === 0;

  const remoteEnrolled = rejoins || tailnetKey.trim() !== "";
  const marks = backupMarks(restoreCard, {
    rejoins,
    selfHosted: settings.selfHosted,
    controlServer,
    encryption: encrypting,
    fightMode: fighting,
    sshMode,
  });

  const imageLabel =
    sourceKind === "aircast"
      ? release
        ? `Aircast OS ${release.version} for ${board?.name ?? "this board"}`
        : "No release"
      : (localFileName ?? "No file");

  const summary = buildSummary({
    image: imageLabel,
    aircastd: aircastdQuery.data ?? null,
    hostname,
    ssid,
    noWifi,
    wifilessBoard:
      sourceKind === "aircast" && board !== null && !board.builtinWifi
        ? board.name
        : null,
    authKey: tailnetKey,
    keyWrittenTo: settings.authKeyWrittenTo,
    controlServer: tailnetServer,
    sshMode,
    sshKey,
    detectedKeys,
    devicePassword,
    encryption: encrypting
      ? { tang: tangServers.some(isCaptured), cloud: cloudOn }
      : null,
    fightMode: fighting,
    restore:
      restore === null
        ? null
        : {
            picked: restore.backup !== null,
            backup: restoreBackup,
            sameDrone: restore.sameDrone,
          },
    rejoins: rejoins ? (restoreBackup?.hostname ?? "") : null,
    marks,
  });

  async function handlePickLocal() {
    const path = await pickLocalImage();
    if (path) update({ localImagePath: path, useLocalImage: true });
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

    const tailscale: TailscaleConfig | null = joinsTailnet(
      tailnetKey,
      tailnetServer,
    )
      ? { controlServer: tailnetServer.trim(), authKey: tailnetKey.trim() }
      : null;

    const access = buildAccess(sshMode, sshKey, devicePassword);

    const capturedServers = tangServers
      .filter(isCaptured)
      .map((s) => ({ url: s.url.trim(), adv: s.adv }));
    const keyServers = capturedServers.length + (cloudOn ? 1 : 0);
    const encryption: EncryptionConfig | null =
      encrypting &&
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
      headscaleUser: headscaleUser.trim(),
      access,
      encryption,
      cloudKeyServer: encrypting && cloudOn,
      fightMode: fighting,
      restore:
        restore === null ||
        restore.backup === null ||
        restore.sameDrone === null
          ? null
          : { backup: restore.backup, sameDrone: restore.sameDrone },
      cloudAccount: linkedAccount(cloudLink),
    };
    setLastVars(vars);
    flashMutation.mutate(vars);
  }

  function answerTelemetry(share: boolean) {
    void answerConsent(share, update, track);
  }

  function handleCancel() {
    setCancelled(true);
    void cancelFlash().catch(() => undefined);
  }

  function retryFlash(vars: FlashVars) {
    setCancelled(false);
    flashMutation.mutate({ ...vars, jobId: randomHex(8) });
  }

  function backTo(target: WizardStep) {
    flashMutation.reset();
    setProgress({ phase: "idle" });
    setCancelled(false);
    setStep(target);
    void queryClient.invalidateQueries({ queryKey: ["devices"] });
  }

  function flashAnother() {
    setRestore(null);
    setHostname(nextFreeHostname(hostname, takenHostnames));
    backTo(STEP.os);
  }
  const writing = flashMutation.isPending;
  const stopped = cancelled && flashMutation.isError;
  const signedOut =
    flashMutation.isError && lastVars?.cloudKeyServer === true && !cloudSignedIn;
  const writesBoard = isUsbTarget(lastVars?.targetDisk ?? "");
  const writtenBackup =
    backups.find((b) => b.id === lastVars?.restore?.backup) ?? null;
  const writtenRestore =
    writtenBackup === null
      ? null
      : {
          host: writtenBackup.hostname,
          rejoins: rejoinsTailnet(
            writtenBackup.card,
            lastVars?.restore?.sameDrone ?? null,
          ),
        };
  const status = writing
    ? "Writing in progress"
    : flashMutation.isSuccess
      ? writesBoard
        ? "Board written"
        : "Card written"
      : stopped
        ? "Write stopped"
        : flashMutation.isError
          ? "Write failed"
          : step === STEP.storage && canProceed
            ? "Ready to write"
            : "Configure your device";

  const stepName = (Object.keys(STEP) as (keyof typeof STEP)[]).find((name) => STEP[name] === step) ?? "os";
  const textSetters: Record<TextField, (value: string) => void> = {
    hostname: setHostname,
    ssid: setSsid,
    wifiPassword: setPassword,
    authKey: setAuthKey,
    devicePassword: setDevicePassword,
  };
  const toggleSetters: Record<ToggleField, (on: boolean) => void> = {
    noWifi: setNoWifi,
    remoteAccess: (on) => void update({ remoteAccess: on }),
    encryption: enableEncryption,
    fightMode: enableFightMode,
  };
  const gotoPage = (page: DebugPage) => {
    if (page === "devices" || page === "cloud") {
      setFleetView(page);
      return;
    }
    setFleetView(null);
    setStep(STEP[page]);
  };
  const debugControls: DebugControls = {
    view: {
      page: fleetView ?? (asksConsent(loaded, settings.telemetry) ? "consent" : stepName),
      boards: boards.map((b) => b.id),
      board: settings.board,
      sourceKind,
      versions: releases.map((r) => r.version),
      version: release?.version ?? null,
      hostname,
      ssid,
      noWifi,
      wifiPasswordSet: password !== "",
      remoteAccess,
      authKeySet: authKey !== "",
      sshMode,
      devicePasswordSet: devicePassword !== "",
      encryption: encryptEnabled,
      fightMode,
      disks: targets.map((d) => ({ path: d.path, name: d.name, size: d.size })),
      disk: selectedDisk,
      problems: storageProblems.map((p) => p.message),
      canFlash: canProceed,
      status,
      progress,
      writing,
      written: flashMutation.isSuccess,
      error: flashMutation.isError ? errorMessage(flashMutation.error, "Flashing failed.") : null,
    },
    consent: answerTelemetry,
    goto: gotoPage,
    board: (id) => {
      const next = boards.find((b) => b.id === id);
      if (next) setBoard(next);
    },
    version: setSelectedVersion,
    disk: setPickedDisk,
    text: (field, value) => textSetters[field](value),
    toggle: (field, on) => toggleSetters[field](on),
    sshMode: setSshMode,
    flash: handleFlash,
    cancel: handleCancel,
  };
  const debug = useRef(debugControls);
  debug.current = debugControls;
  useEffect(() => {
    const unlisten = listenDebugRequests((request) => runDebugRequest(request, debug.current));
    return () => void unlisten.then((stop) => stop());
  }, []);

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
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
          status={status}
        />

        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          {fleetView === "devices" ? (
            <DevicesView
              onFlash={() => {
                setFleetView(null);
                setStep(STEP.os);
              }}
              onRestore={(backup) => {
                setFleetView(null);
                chooseRestore({ backup, sameDrone: null });
                setStep(STEP.network);
              }}
              background={{
                on: settings.backgroundBackups,
                set: (on) =>
                  setBackgroundBackups(on).then((tray) =>
                    update({ backgroundBackups: on }).then(() => tray),
                  ),
                decline: () => void update({ backgroundBackups: false }),
              }}
            />
          ) : fleetView === "cloud" ? (
            <CloudCardsView />
          ) : asksConsent(loaded, settings.telemetry) ? (
            <StepConsent onAnswer={answerTelemetry} />
          ) : step === STEP.os ? (
            <StepImage
              boards={boards}
              board={board}
              onBoard={setBoard}
              sourceKind={sourceKind}
              onSourceKind={setSourceKind}
              release={release}
              releases={releases}
              onSelectVersion={setSelectedVersion}
              releaseLoading={releasesQuery.isLoading}
              releaseError={releaseError}
              localFileName={localFileName}
              onPickLocal={handlePickLocal}
              cacheStatus={cacheStatus}
              onNext={() => setStep(STEP.network)}
            />
          ) : step === STEP.network ? (
            <StepNetwork
              board={board}
              ssid={ssid}
              onSsid={setSsid}
              knownNetworks={networkChoices(
                wifiQuery.data?.nearby ?? [],
                wifiQuery.data?.known ?? [],
              )}
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
              restore={restore}
              onRestore={chooseRestore}
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
                keyWrittenTo: settings.authKeyWrittenTo,
                enabled: remoteAccess,
                onEnabled: (remoteAccess) => update({ remoteAccess }),
                selfHosted: settings.selfHosted,
                onSelfHosted: (selfHosted) => update({ selfHosted }),
                headscaleUser,
                onHeadscaleUser: (headscaleUser) => update({ headscaleUser }),
                rejoins: rejoins ? (restoreBackup?.hostname ?? "") : null,
              }}
              marks={marks}
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
              encryptionAvailable={encryptionAvailable}
              encryption={{
                enabled: encrypting,
                onEnabled: enableEncryption,
                servers: tangServers,
                onServers: setTangServers,
                threshold,
                onThreshold: setThreshold,
                cloud: cloudOn,
                onCloud: setCloudKeyServer,
                recoveryKey,
                recoverySaved,
                onRecoverySaved: setRecoverySaved,
                wifiUplink: !noWifi && ssid.trim() !== "",
                hostname,
              }}
              fight={{
                enabled: fighting,
                available: fightAvailable,
                onEnabled: enableFightMode,
              }}
              onBack={() => setStep(STEP.network)}
              onNext={() => setStep(STEP.storage)}
            />
          ) : step === STEP.storage ? (
            <StepStorage
              board={board}
              runningBoards={runningBoardsQuery.data ?? []}
              devices={devices}
              devicesLoading={devicesQuery.isLoading}
              selectedDisk={selectedDisk}
              onSelectDisk={setPickedDisk}
              requiredCardBytes={requiredCardBytes}
              summary={summary}
              cloudLink={cloudLink}
              onCloudLink={(on) => void update({ cloudLink: on })}
              problems={storageProblems}
              onBack={() => setStep(STEP.access)}
              onEditStep={setStep}
              onFlash={handleFlash}
            />
          ) : (
            <JobView
              hostname={(lastVars?.hostname ?? hostname).trim()}
              board={sourceKind === "aircast" ? board : null}
              usb={writesBoard}
              noWifi={noWifi}
              remoteEnrolled={remoteEnrolled}
              fightMode={fighting}
              restore={writtenRestore}
              retire={
                writtenBackup !== null &&
                lastVars?.restore?.sameDrone === true &&
                flashMutation.data?.cloudCard
                  ? {
                      hostnames: [writtenBackup.hostname, lastVars.hostname],
                      keep: flashMutation.data.cloudCard,
                    }
                  : null
              }
              encryption={flashMutation.data?.encryption ?? null}
              cloud={flashMutation.data?.cloud ?? null}
              aircastd={flashMutation.data?.aircastd ?? null}
              success={flashMutation.isSuccess}
              cancelled={stopped}
              error={
                flashMutation.isError
                  ? errorMessage(flashMutation.error, "Flashing failed.")
                  : null
              }
              cardTouched={cardTouched}
              signedOut={signedOut}
              progress={progress}
              onCancel={handleCancel}
              onRetry={lastVars === null ? null : () => retryFlash(lastVars)}
              onChangeSettings={() =>
                backTo(signedOut ? STEP.access : STEP.storage)
              }
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

import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";

import { readSettings, updateSettings } from "@/api";
import type { Settings } from "@/types";

const KEY = ["settings"];

const EMPTY: Settings = {
  ssid: null,
  hostname: null,
  wifiPassword: "",
  controlServer: "",
  authorizedKey: null,
  noWifi: false,
  telemetry: null,
  installId: null,
  flashedHostnames: [],
  tangUrls: [],
  tangThreshold: 1,
  tangCloud: false,
  authKey: "",
  remoteAccess: false,
  selfHosted: false,
  sshMode: null,
  devicePassword: "",
  fightMode: false,
  encryption: false,
  useLocalImage: false,
  localImagePath: null,
};

export function useSettings(): {
  settings: Settings;
  update: (patch: Partial<Settings>) => Promise<boolean>;
  loaded: boolean;
} {
  const query = useQuery({
    queryKey: KEY,
    queryFn: readSettings,
    staleTime: Infinity,
  });
  const [edited, setEdited] = useState<Settings | undefined>(undefined);
  const current = edited ?? query.data;

  const persist = useMutation({ mutationFn: updateSettings });

  return {
    settings: current ?? EMPTY,
    loaded: current !== undefined,
    update: (patch) => {
      if (current === undefined) return Promise.resolve(false);
      setEdited((prev) => ({ ...(prev ?? current), ...patch }));
      return persist.mutateAsync(patch).then(
        () => true,
        () => false,
      );
    },
  };
}

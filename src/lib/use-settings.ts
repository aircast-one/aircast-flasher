import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";

import { readSettings, writeSettings } from "@/api";
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
};

export function useSettings(): {
  settings: Settings;
  update: (patch: Partial<Settings>) => void;
} {
  const query = useQuery({
    queryKey: KEY,
    queryFn: readSettings,
    staleTime: Infinity,
  });

  const persist = useMutation({ mutationFn: writeSettings });
  const [edited, setEdited] = useState<Settings | null>(null);
  const settings = edited ?? query.data ?? EMPTY;

  return {
    settings,
    update: (patch) => {
      if (query.data === undefined) return;
      const next = { ...settings, ...patch };
      setEdited(next);
      persist.mutate(next);
    },
  };
}

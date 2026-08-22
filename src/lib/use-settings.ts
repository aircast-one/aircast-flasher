import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

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
};

export function useSettings(): {
  settings: Settings;
  update: (patch: Partial<Settings>) => void;
} {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: KEY,
    queryFn: readSettings,
    staleTime: Infinity,
  });

  const persist = useMutation({ mutationFn: writeSettings });
  const settings = query.data ?? EMPTY;

  return {
    settings,
    update: (patch) => {
      const next = { ...settings, ...patch };
      queryClient.setQueryData(KEY, next);
      persist.mutate(next);
    },
  };
}

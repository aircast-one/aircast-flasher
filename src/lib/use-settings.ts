import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

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

  const persist = useMutation({ mutationFn: updateSettings });

  return {
    settings: query.data ?? EMPTY,
    update: (patch) => {
      if (query.data === undefined) return;
      queryClient.setQueryData<Settings>(KEY, { ...query.data, ...patch });
      persist.mutate(patch);
    },
  };
}

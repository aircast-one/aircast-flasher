import { useQuery } from "@tanstack/react-query";

import { localTailscale } from "@/api";
import type { LocalTailscale } from "@/types";

const POLL_INTERVAL_MS = 5000;

export function useLocalTailscale(): LocalTailscale | null {
  const query = useQuery({
    queryKey: ["local-tailscale"],
    queryFn: localTailscale,
    refetchInterval: POLL_INTERVAL_MS,
    staleTime: POLL_INTERVAL_MS,
  });
  return query.data ?? null;
}

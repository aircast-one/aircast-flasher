import { useQuery } from "@tanstack/react-query";

import { listBackups } from "@/api";
import type { Backup } from "@/types";

export const BACKUPS_KEY = ["backups"];

export function useBackups(): Backup[] {
  return useQuery({ queryKey: BACKUPS_KEY, queryFn: listBackups }).data ?? [];
}

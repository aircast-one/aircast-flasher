import type { Release } from "../types";

export function releaseLabel(
  release: Pick<Release, "version" | "prerelease">,
): string {
  if (!release.prerelease) return "";
  return release.version.includes("-beta.") ? " · beta" : " · pre-release";
}

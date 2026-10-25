export const asksConsent = (loaded: boolean, telemetry: boolean | null) =>
  loaded && telemetry === null;

export const answerConsent = (
  share: boolean,
  save: (patch: { telemetry: boolean }) => Promise<boolean>,
  send: (event: string) => void,
): Promise<void> =>
  save({ telemetry: share }).then((saved) =>
    saved && share ? send("telemetry_enabled") : undefined,
  );

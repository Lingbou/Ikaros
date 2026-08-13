import { DEFAULT_PROFILE_USERNAME } from "../shared/platform";

export interface LocalProfile {
  readonly name: string;
  readonly initials: string;
}

export const LOCAL_PROFILE: LocalProfile = Object.freeze({
  name: DEFAULT_PROFILE_USERNAME,
  initials: "US",
});

export function profileInitials(username: string): string {
  const words = username.trim().split(/\s+/u).filter(Boolean);
  if (!words.length) return LOCAL_PROFILE.initials;

  const initials =
    words.length === 1
      ? Array.from(words[0]).slice(0, 2)
      : [Array.from(words[0])[0], Array.from(words.at(-1) ?? "")[0]];

  return initials.filter(Boolean).join("").toUpperCase() || LOCAL_PROFILE.initials;
}

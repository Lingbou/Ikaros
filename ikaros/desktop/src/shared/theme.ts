import type {
  ColorSchemePreference,
  ThemePreferences,
  UiPreferences
} from "./platform";

interface RgbColor {
  r: number;
  g: number;
  b: number;
}

export interface DerivedThemeColors {
  accent: string;
  background: string;
  border: string;
  borderSoft: string;
  canvas: string;
  foreground: string;
  menu: string;
  muted: string;
  mutedStrong: string;
  panel: string;
  panelHover: string;
  panelRaised: string;
  panelSelected: string;
  separator: string;
  shadow: string;
  sidebar: string;
  sidebarBottom: string;
  sidebarTranslucent: string;
  surfaceHover: string;
  titlebar: string;
}

const HEX_COLOR = /^#[0-9a-f]{6}$/i;

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}

export function isHexColor(value: unknown): value is string {
  return typeof value === "string" && HEX_COLOR.test(value);
}

export function normalizeHexColor(value: string): string {
  return value.toLowerCase();
}

export function hexToRgb(value: string): RgbColor {
  if (!isHexColor(value)) {
    throw new TypeError(`Invalid hex color: ${value}`);
  }
  return {
    r: Number.parseInt(value.slice(1, 3), 16),
    g: Number.parseInt(value.slice(3, 5), 16),
    b: Number.parseInt(value.slice(5, 7), 16)
  };
}

function rgbToHex({ r, g, b }: RgbColor): string {
  const channel = (value: number) => Math.round(clamp(value, 0, 255)).toString(16).padStart(2, "0");
  return `#${channel(r)}${channel(g)}${channel(b)}`;
}

export function mixHexColors(from: string, to: string, amount: number): string {
  const start = hexToRgb(from);
  const end = hexToRgb(to);
  const weight = clamp(amount, 0, 1);
  return rgbToHex({
    r: start.r + (end.r - start.r) * weight,
    g: start.g + (end.g - start.g) * weight,
    b: start.b + (end.b - start.b) * weight
  });
}

export function hexWithAlpha(value: string, alpha: number): string {
  const { r, g, b } = hexToRgb(value);
  return `rgba(${r}, ${g}, ${b}, ${clamp(alpha, 0, 1).toFixed(3)})`;
}

export function readableTextColor(background: string): "#111111" | "#ffffff" {
  const { r, g, b } = hexToRgb(background);
  const linear = [r, g, b].map((channel) => {
    const normalized = channel / 255;
    return normalized <= 0.03928
      ? normalized / 12.92
      : ((normalized + 0.055) / 1.055) ** 2.4;
  });
  const luminance = 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
  return luminance > 0.48 ? "#111111" : "#ffffff";
}

export function effectiveColorScheme(
  preference: ColorSchemePreference,
  systemPrefersDark: boolean
): "light" | "dark" {
  return preference === "system" ? (systemPrefersDark ? "dark" : "light") : preference;
}

export function activeThemePreferences(
  preferences: Readonly<UiPreferences>,
  systemPrefersDark: boolean
): ThemePreferences {
  return effectiveColorScheme(preferences.colorScheme, systemPrefersDark) === "dark"
    ? preferences.darkTheme
    : preferences.lightTheme;
}

export function deriveThemeColors(theme: Readonly<ThemePreferences>): DerivedThemeColors {
  const contrast = clamp(theme.contrast, 0, 100) / 100;
  const background = normalizeHexColor(theme.background);
  const foreground = normalizeHexColor(theme.foreground);
  const accent = normalizeHexColor(theme.accent);
  const backgroundIsDark = readableTextColor(background) === "#ffffff";

  return {
    accent,
    background,
    border: mixHexColors(background, foreground, 0.1 + contrast * 0.09),
    borderSoft: mixHexColors(background, foreground, 0.07 + contrast * 0.07),
    canvas: background,
    foreground,
    menu: mixHexColors(background, foreground, 0.085 + contrast * 0.025),
    muted: mixHexColors(background, foreground, 0.4 + contrast * 0.25),
    mutedStrong: mixHexColors(background, foreground, 0.56 + contrast * 0.23),
    panel: mixHexColors(background, foreground, 0.035 + contrast * 0.035),
    panelHover: mixHexColors(background, foreground, 0.07 + contrast * 0.065),
    panelRaised: mixHexColors(background, foreground, 0.055 + contrast * 0.07),
    panelSelected: mixHexColors(background, foreground, 0.085 + contrast * 0.075),
    separator: mixHexColors(background, foreground, 0.06 + contrast * 0.05),
    shadow: hexWithAlpha(backgroundIsDark ? "#000000" : "#111111", backgroundIsDark ? 0.48 : 0.16),
    sidebar: mixHexColors(background, foreground, 0.025 + contrast * 0.03),
    sidebarBottom: mixHexColors(background, accent, 0.055 + contrast * 0.07),
    sidebarTranslucent: hexWithAlpha(
      mixHexColors(background, foreground, 0.025 + contrast * 0.03),
      0.9
    ),
    surfaceHover: hexWithAlpha(foreground, 0.045 + contrast * 0.04),
    titlebar: mixHexColors(background, foreground, 0.025 + contrast * 0.025)
  };
}

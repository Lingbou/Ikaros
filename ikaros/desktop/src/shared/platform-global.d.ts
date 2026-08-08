import type { IkarosDesktopApi } from "./platform";

declare global {
  interface Window {
    readonly ikarosDesktop: IkarosDesktopApi;
  }
}

export {};

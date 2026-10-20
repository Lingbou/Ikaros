import * as Dialog from "@radix-ui/react-dialog";
import { CheckCircle2 } from "lucide-react";
import { useEffect, useState } from "react";

import { useTranslation } from "../i18n";
import { useAppStore } from "../store";
import { CustomProviderForm, DeepSeekConnectForm } from "./ProviderModelSettings";

type ProviderMode = "deepseek" | "custom";
type OnboardingPhase = "checking" | "show" | "hidden";

/** First-run setup: connect an OpenAI-compatible provider before the first chat. */
export function OnboardingDialog() {
  const { t } = useTranslation();
  const connectionStatus = useAppStore((state) => state.runtimeConnectionStatus);
  const providerCatalogStatus = useAppStore((state) => state.providerCatalogStatus);
  const providers = useAppStore((state) => state.providers);
  const configureProvider = useAppStore((state) => state.configureProvider);
  const discoverDeepSeekModels = useAppStore((state) => state.discoverDeepSeekModels);
  const loadProviderCatalog = useAppStore((state) => state.loadProviderCatalog);
  const [completed, setCompleted] = useState<boolean | null>(null);
  const [phase, setPhase] = useState<OnboardingPhase>("checking");
  const [connected, setConnected] = useState(false);
  const [mode, setMode] = useState<ProviderMode>("deepseek");

  useEffect(() => {
    const api = window.ikarosDesktop;
    if (!api) {
      setCompleted(true);
      return;
    }
    let disposed = false;
    void api.preferences
      .get()
      .then((preferences) => {
        if (!disposed) setCompleted(preferences.onboardingCompleted);
      })
      .catch(() => {
        if (!disposed) setCompleted(true);
      });
    return () => {
      disposed = true;
    };
  }, []);

  const ready = connectionStatus === "connected" && providerCatalogStatus === "ready";
  const hasConfiguredProvider = providers.some((provider) => provider.configured);

  useEffect(() => {
    if (phase !== "checking" || completed === null || !ready) return;
    if (completed === false && !hasConfiguredProvider) {
      setPhase("show");
      return;
    }
    if (completed === false) {
      // A provider was configured elsewhere (or before onboarding existed):
      // remember that setup is done and stay out of the way.
      void window.ikarosDesktop?.preferences
        .update({ onboardingCompleted: true })
        .catch(() => undefined);
    }
    setPhase("hidden");
  }, [phase, completed, ready, hasConfiguredProvider]);

  const finish = () => {
    setPhase("hidden");
    void window.ikarosDesktop?.preferences
      .update({ onboardingCompleted: true })
      .catch(() => undefined);
  };

  const connectedHandler = () => {
    setConnected(true);
    void loadProviderCatalog();
  };

  if (phase !== "show") return null;

  return (
    <Dialog.Root
      open
      onOpenChange={(next) => {
        if (!next) finish();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay
          data-testid="onboarding-overlay"
          className="fixed inset-0 z-[130] bg-black/55 backdrop-blur-[2px]"
        />
        <Dialog.Content
          aria-describedby="onboarding-description"
          className="glass-menu fixed left-1/2 top-1/2 z-[140] flex max-h-[calc(100vh-40px)] w-[min(640px,calc(100vw-32px))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-2xl"
        >
          {connected ? (
            <div className="flex flex-col items-center px-8 py-10 text-center">
              <CheckCircle2 size={34} aria-hidden="true" className="text-[var(--accent)]" />
              <Dialog.Title className="mt-4 text-[16px] font-semibold leading-6 text-[var(--text)]">
                {t("onboarding.connectedTitle")}
              </Dialog.Title>
              <Dialog.Description
                id="onboarding-description"
                className="mt-2 max-w-[380px] text-[12px] leading-5 text-[var(--muted)]"
              >
                {t("onboarding.connectedDescription")}
              </Dialog.Description>
              <button
                type="button"
                autoFocus
                onClick={finish}
                className="mt-6 h-9 rounded-xl bg-[var(--text)] px-5 text-[12px] font-medium text-[var(--canvas)] transition-transform hover:scale-[1.02]"
              >
                {t("onboarding.start")}
              </button>
            </div>
          ) : (
            <>
              <div className="shrink-0 border-b border-[var(--separator)] px-6 py-5">
                <Dialog.Title className="text-[16px] font-semibold leading-6 text-[var(--text)]">
                  {t("onboarding.title")}
                </Dialog.Title>
                <Dialog.Description
                  id="onboarding-description"
                  className="mt-1.5 text-[12px] leading-5 text-[var(--muted)]"
                >
                  {t("onboarding.description")}
                </Dialog.Description>
              </div>
              <div className="app-scrollbar min-h-0 flex-1 overflow-y-auto px-6 py-5">
                <div
                  role="tablist"
                  aria-label={t("onboarding.title")}
                  className="mb-5 flex gap-1 rounded-xl border border-[var(--border-soft)] bg-[var(--panel)] p-1"
                >
                  {(["deepseek", "custom"] as const).map((candidate) => (
                    <button
                      key={candidate}
                      type="button"
                      role="tab"
                      aria-selected={mode === candidate}
                      onClick={() => setMode(candidate)}
                      className={
                        mode === candidate
                          ? "h-8 flex-1 rounded-lg bg-[var(--surface-hover)] text-[12px] font-medium text-[var(--text)] transition-colors"
                          : "h-8 flex-1 rounded-lg text-[12px] font-medium text-[var(--muted)] transition-colors hover:text-[var(--text)]"
                      }
                    >
                      {t(
                        candidate === "deepseek"
                          ? "onboarding.deepSeekTab"
                          : "onboarding.customTab",
                      )}
                    </button>
                  ))}
                </div>
                {mode === "deepseek" ? (
                  <DeepSeekConnectForm
                    existingModels={[]}
                    onConnected={connectedHandler}
                    onConnect={configureProvider}
                    onDiscoverModels={discoverDeepSeekModels}
                  />
                ) : (
                  <CustomProviderForm
                    onConfigured={connectedHandler}
                    onSubmit={configureProvider}
                  />
                )}
              </div>
              <div className="shrink-0 border-t border-[var(--separator)] px-6 py-3 text-center">
                <button
                  type="button"
                  onClick={finish}
                  className="text-[11px] leading-4 text-[var(--muted)] transition-colors hover:text-[var(--text)]"
                >
                  {t("onboarding.skip")}
                </button>
              </div>
            </>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

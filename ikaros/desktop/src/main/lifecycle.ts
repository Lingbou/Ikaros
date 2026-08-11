export interface BeforeQuitEvent {
  preventDefault(): void;
}

export interface StoppableRuntimeHost {
  stop(): Promise<void>;
}

export type RuntimeShutdownState = "idle" | "pending" | "ready";

export function deferQuitForRuntimeShutdown(
  event: BeforeQuitEvent,
  runtimeHost: StoppableRuntimeHost | undefined,
  shutdownState: RuntimeShutdownState,
  quit: () => void
): RuntimeShutdownState {
  if (!runtimeHost || shutdownState === "ready") {
    return shutdownState;
  }
  event.preventDefault();
  if (shutdownState === "pending") {
    return shutdownState;
  }
  void runtimeHost.stop().then(quit, quit);
  return "pending";
}

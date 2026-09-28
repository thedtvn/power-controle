import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { emit as tauriEmit, listen as tauriListen } from "@tauri-apps/api/event";
import { getCurrentWindow as tauriGetCurrentWindow } from "@tauri-apps/api/window";

export const isTauriRuntime = "__TAURI_INTERNALS__" in window;

const unavailable = () => Promise.reject(
  new Error("This screen must be opened through `cargo tauri dev`, not the Vite URL directly."),
);

export const invoke = isTauriRuntime ? tauriInvoke : unavailable;
export const listen = isTauriRuntime ? tauriListen : unavailable;
export const emit = isTauriRuntime ? tauriEmit : unavailable;
export const getCurrentWindow = isTauriRuntime
  ? tauriGetCurrentWindow
  : () => ({ close: unavailable });

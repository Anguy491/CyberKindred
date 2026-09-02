import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type IpcUnlisten = UnlistenFn;

/** The sole injectable boundary between product UI and Tauri IPC. */
export interface IpcTransport {
  invoke<Response>(command: string, args: Readonly<Record<string, unknown>>): Promise<Response>;
  listen<Payload>(eventName: string, handler: (payload: Payload) => void): Promise<IpcUnlisten>;
}

/** Production transport; tests inject a hermetic in-memory replacement. */
export const tauriIpcTransport: IpcTransport = {
  invoke<Response>(command: string, args: Readonly<Record<string, unknown>>) {
    return invoke<Response>(command, args);
  },
  listen<Payload>(eventName: string, handler: (payload: Payload) => void) {
    return listen<Payload>(eventName, (event) => handler(event.payload));
  },
};

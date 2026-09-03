import { YouView, type YouIpc } from "../features/you";

export function YouPage({ ipc }: { readonly ipc: YouIpc }) {
  return <YouView ipc={ipc} />;
}

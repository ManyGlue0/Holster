import { memo, useEffect, useState } from "react";
import type { AppView } from "./api";

// Lucide icons by name (from app manifests), rendered from icon nodes. The
// data loads lazily so the tab paints instantly.

type IconNode = [string, Record<string, string>][];
type Nodes = Record<string, IconNode>;

let nodes: Nodes | null = null;
let loading: Promise<void> | null = null;
const waiters = new Set<() => void>();

export function loadIcons(): Promise<void> {
  loading ??= import("./lucide-nodes.json").then((n) => {
    nodes = n.default as unknown as Nodes;
    waiters.forEach((w) => w());
    waiters.clear();
  });
  return loading;
}

function useIconsReady() {
  const [ready, setReady] = useState(nodes !== null);
  useEffect(() => {
    if (ready) return;
    const w = () => setReady(true);
    waiters.add(w);
    if (nodes) w();
    loadIcons();
    return () => void waiters.delete(w);
  }, [ready]);
  return ready;
}

export const LucideIcon = memo(function LucideIcon({
  name,
  size = 18,
  strokeWidth = 1.9,
}: {
  name: string;
  size?: number;
  strokeWidth?: number;
}) {
  useIconsReady();
  const node = nodes?.[name] ?? nodes?.["app-window"];
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      {node?.map(([tag, attrs], i) => {
        const Tag = tag as unknown as React.ElementType;
        return <Tag key={i} {...attrs} />;
      })}
    </svg>
  );
});

/** An app's icon on a tile of `tile` px: its Lucide icon at `size` px, or
 *  its image covering the app's chosen share of the tile (default 80%). */
export function AppIcon({
  app,
  size = 22,
  tile,
}: {
  app: Pick<AppView, "lucide" | "image" | "imageSize">;
  size?: number;
  tile: number;
}) {
  if (app.image) {
    const px = Math.round((tile * app.imageSize) / 100);
    return <img className="app-image" src={app.image} width={px} height={px} alt="" draggable={false} />;
  }
  return <LucideIcon name={app.lucide ?? "app-window"} size={size} strokeWidth={1.9} />;
}

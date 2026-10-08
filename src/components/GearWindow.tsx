import { AnimatePresence, motion } from "framer-motion";
import { Settings } from "lucide-react";
import { useState } from "react";
import { api } from "../lib/api";
import { DOCK_PAD_L, DOCK_W, GEAR_H, TILE } from "../lib/layout";
import { rowSpring, softSpring } from "../lib/motion";
import { useStore } from "../lib/store";

/** The settings card, detached below the dock. It never scrolls. */
export function GearWindow() {
  const mode = useStore((s) => s.shell.mode);
  const overhang = useStore((s) => s.shell.overhang);
  const [hover, setHover] = useState(false);
  const visible = mode === "opening" || mode === "dock";

  return (
    <div className="card-root">
      <div className="card-veil" />
      <AnimatePresence>
        {visible && (
          <motion.button
            key="gear"
            className="gear"
            aria-label="Holster settings"
            style={{ width: DOCK_W, height: GEAR_H, right: overhang, paddingLeft: DOCK_PAD_L }}
            initial={{ opacity: 0, x: 12 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, transition: { duration: 0.07 } }}
            transition={{ ...rowSpring, delay: 0.16 }}
            onPointerEnter={() => setHover(true)}
            onPointerLeave={() => setHover(false)}
            onClick={() => api.openPanel()}
          >
            <span className="gear-pill" style={{ width: TILE }}>
              <motion.span className="gear-icon" animate={{ rotate: hover ? 45 : 0 }} transition={softSpring}>
                <Settings size={18} />
              </motion.span>
            </span>
          </motion.button>
        )}
      </AnimatePresence>
    </div>
  );
}

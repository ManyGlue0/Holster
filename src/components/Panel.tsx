import { AnimatePresence, motion } from "framer-motion";
import { api } from "../lib/api";
import { conceal, reveal } from "../lib/motion";
import { useStore } from "../lib/store";
import { SettingsPage } from "./Settings";

/** The settings panel. Content is laid out at the panel's final size and
 *  anchored to the top-right, so while the window morphs it is revealed, not
 *  reflowed (the Anotes panel technique). */
export function Panel() {
  const shell = useStore((s) => s.shell);
  const leaving = useStore((s) => s.panelLeaving);
  const visible = shell.mode === "panel" && !leaving;

  return (
    <div className="panel-root" style={{ width: `max(100%, ${shell.panelWidth}px)`, height: `max(100%, ${shell.panelHeight}px)` }}>
      <motion.div
        className="panel"
        initial={false}
        animate={visible ? { opacity: 1, filter: "blur(0px)", scale: 1 } : { opacity: 0, filter: "blur(6px)", scale: 0.985 }}
        transition={visible ? reveal : conceal}
        style={{ pointerEvents: visible ? "auto" : "none" }}
      >
        <button className="edge-handle" title="Close (Esc)" aria-label="Close" onClick={() => api.close()} />
        <SettingsPage />
        <Toasts />
      </motion.div>
    </div>
  );
}

function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);
  return (
    <div className="toasts">
      <AnimatePresence initial={false}>
        {toasts.map((t) => (
          <motion.div
            key={t.id}
            layout
            className="toast"
            initial={{ opacity: 0, y: 16, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 8, scale: 0.97, transition: { duration: 0.15 } }}
            transition={{ type: "spring", stiffness: 500, damping: 36 }}
          >
            <span>{t.text}</span>
            {t.action && (
              <button
                onClick={() => {
                  t.action!.run();
                  dismiss(t.id);
                }}
              >
                {t.action.label}
              </button>
            )}
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}

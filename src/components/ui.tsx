import { AnimatePresence, motion } from "framer-motion";
import { Minus, Pin, PinOff, Plus } from "lucide-react";
import { forwardRef, useState } from "react";
import { spring } from "../lib/motion";
import { useStore } from "../lib/store";

// ---- buttons --------------------------------------------------------------

type IconButtonProps = React.ButtonHTMLAttributes<HTMLButtonElement> & { label: string; active?: boolean };

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, className, active, children, ...rest },
  ref,
) {
  return (
    <button ref={ref} aria-label={label} title={label} className={`icon-btn ${active ? "active" : ""} ${className ?? ""}`} {...rest}>
      {children}
    </button>
  );
});

/** Keeps the panel open on click outside, for this session. Pinned, it stays
 *  lit and hovering previews the unpin. */
export function PinButton() {
  const pinned = useStore((s) => s.pinned);
  const [hover, setHover] = useState(false);
  const Icon = pinned && hover ? PinOff : Pin;
  return (
    <IconButton
      label={pinned ? "Unpin (closes on click outside)" : "Pin open"}
      active={pinned}
      aria-pressed={pinned}
      onPointerEnter={() => setHover(true)}
      onPointerLeave={() => setHover(false)}
      onClick={() => useStore.setState({ pinned: !pinned })}
    >
      <AnimatePresence mode="popLayout" initial={false}>
        <motion.span
          key={`${pinned}-${Icon === PinOff}`}
          className="pin-icon"
          initial={{ opacity: 0, scale: 0.6, rotate: pinned ? -30 : 30 }}
          animate={{ opacity: 1, scale: 1, rotate: pinned ? 0 : 45 }}
          exit={{ opacity: 0, scale: 0.6, transition: { duration: 0.1 } }}
          transition={spring}
        >
          <Icon size={16} fill={pinned && Icon === Pin ? "currentColor" : "none"} />
        </motion.span>
      </AnimatePresence>
    </IconButton>
  );
}

export function Button({
  variant = "default",
  className,
  ...rest
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "default" | "primary" | "ghost" | "danger" }) {
  return <button className={`btn btn-${variant} ${className ?? ""}`} {...rest} />;
}

// ---- small controls ------------------------------------------------------------

export function Toggle({ on, onChange, label }: { on: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button role="switch" aria-checked={on} aria-label={label} className={`toggle ${on ? "on" : ""}`} onClick={() => onChange(!on)}>
      {/* Animates its offset in the track, not a layout animation, which would also chase the row while it's dragged. */}
      <motion.span className="toggle-knob" initial={false} animate={{ x: on ? 16 : 0 }} transition={spring} />
    </button>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  id,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  id: string;
}) {
  return (
    <div className="segmented">
      {options.map((o) => (
        <button key={o.value} className={o.value === value ? "on" : ""} onClick={() => onChange(o.value)}>
          {o.value === value && <motion.span layoutId={`seg-${id}`} className="segmented-pill" transition={spring} />}
          <span>{o.label}</span>
        </button>
      ))}
    </div>
  );
}

/** A number in fixed steps, with its unit. */
export function Stepper({
  value,
  onChange,
  min,
  max,
  step,
  unit,
  label,
}: {
  value: number;
  onChange: (v: number) => void;
  min: number;
  max: number;
  step: number;
  unit: string;
  label: string;
}) {
  const set = (v: number) => onChange(Math.max(min, Math.min(max, Math.round(v / step) * step)));
  return (
    <div className="stepper" role="group" aria-label={label}>
      <button aria-label={`Less ${label}`} disabled={value <= min} onClick={() => set(value - step)}>
        <Minus size={13} />
      </button>
      <span className="stepper-value">
        <AnimatePresence mode="popLayout" initial={false}>
          <motion.span key={value} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -6 }} transition={spring}>
            {value}
          </motion.span>
        </AnimatePresence>
        <span className="stepper-unit">{unit}</span>
      </span>
      <button aria-label={`More ${label}`} disabled={value >= max} onClick={() => set(value + step)}>
        <Plus size={13} />
      </button>
    </div>
  );
}

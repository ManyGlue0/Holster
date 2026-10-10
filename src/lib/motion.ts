// Motion presets. Every interaction animates with these.

/** Controls: toggles, segmented pills, popovers, pins. */
export const spring = { type: "spring", stiffness: 520, damping: 38, mass: 0.8 } as const;
/** Larger, calmer movements. */
export const softSpring = { type: "spring", stiffness: 300, damping: 30 } as const;
/** Rows and tiles arriving in a card (staggered by index * 0.04s). */
export const rowSpring = { type: "spring", stiffness: 520, damping: 34 } as const;
/** Page and list movement. */
export const pageSpring = { type: "spring", stiffness: 420, damping: 40 } as const;
/** Content revealing once its window has unfolded. */
export const reveal = { duration: 0.32, ease: [0.16, 1, 0.3, 1] } as const;
/** Content leaving before its window folds away. */
export const conceal = { duration: 0.09, ease: "easeIn" } as const;

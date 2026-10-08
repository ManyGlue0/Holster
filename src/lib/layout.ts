// Shared geometry (CSS px). The backend sizes the windows from the same
// numbers (src-tauri/src/shell.rs), so each window is exactly its card.

export const TAB_W = 14;
export const TILE = 46;
export const TILE_GAP = 8;
export const TILE_STEP = TILE + TILE_GAP;
export const DOCK_PAD_Y = 10;
export const DOCK_PAD_L = 16;
export const DOCK_PAD_R = 10;
export const DOCK_W = DOCK_PAD_L + TILE + DOCK_PAD_R;
export const VISIBLE_TILES = 5;
export const GEAR_H = 50;

import { invoke } from "@tauri-apps/api/core";

/** GUI settings stored in `instances/instance1/config.json`. */
export type GuiConfig = {
  romDir: string;
  selectedRom: string | null;
  scale: number;
  layout: "vertical" | "horizontal";
  /** 0-100 */
  volume: number;
};

export type EmuStatus = {
  running: boolean;
  fps: number;
  gamepads: string[];
  instanceDir: string;
};

/** Width/height of one DS screen. */
export const SCREEN_W = 256;
export const SCREEN_H = 192;

/** DS button bits understood by the backend (`emu_thread::key`). */
export const KEY = {
  A: 1 << 0,
  B: 1 << 1,
  SELECT: 1 << 2,
  START: 1 << 3,
  RIGHT: 1 << 4,
  LEFT: 1 << 5,
  UP: 1 << 6,
  DOWN: 1 << 7,
  R: 1 << 8,
  L: 1 << 9,
  X: 1 << 10,
  Y: 1 << 11,
} as const;

/** Keyboard layout (same as the egui frontend). */
export const KEYMAP: Record<string, number> = {
  ArrowUp: KEY.UP,
  ArrowDown: KEY.DOWN,
  ArrowLeft: KEY.LEFT,
  ArrowRight: KEY.RIGHT,
  KeyX: KEY.A,
  KeyZ: KEY.B,
  KeyS: KEY.X,
  KeyA: KEY.Y,
  KeyQ: KEY.L,
  KeyW: KEY.R,
  Enter: KEY.START,
  Backspace: KEY.SELECT,
};

export const EMU = {
  getConfig: () => invoke<GuiConfig>("get_config"),
  setConfig: (config: GuiConfig) => invoke<void>("set_config", { config }),
  listRoms: (dir: string) => invoke<string[]>("list_roms", { dir }),
  start: (path: string) => invoke<void>("start_emulator", { path }),
  stop: () => invoke<void>("stop_emulator"),
  status: () => invoke<EmuStatus>("get_status"),
  setKeys: (keys: number) => invoke<void>("set_keys", { keys }),
  /** `null` lifts the stylus. */
  setTouch: (pos: { x: number; y: number } | null) =>
    invoke<void>("set_touch", { x: pos?.x ?? null, y: pos?.y ?? null }),

  /**
   * Newest frame after `lastSeq`: `{ seq, upper, lower }` RGBA buffers, or
   * `null` if no new frame was produced yet.
   */
  async frame(lastSeq: number) {
    const buf = await invoke<ArrayBuffer>("get_frame", { lastSeq });
    if (buf.byteLength < 8) return null;
    const seq = Number(new DataView(buf).getBigUint64(0, true));
    const size = SCREEN_W * SCREEN_H * 4;
    return {
      seq,
      upper: new Uint8ClampedArray(buf, 8, size),
      lower: new Uint8ClampedArray(buf, 8 + size, size),
    };
  },
};

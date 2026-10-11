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

/** Analog stick deflection treated as a D-pad press. */
const STICK_THRESHOLD = 0.5;

/**
 * DS button bits pressed on the connected gamepads (Web Gamepad API,
 * "standard" mapping). Face buttons are mapped by position like on the DS
 * (right = A, bottom = B, top = X, left = Y), as in the egui frontend; L/R
 * accept both shoulder buttons and triggers; the left stick doubles as the
 * D-pad.
 */
export const gamepadKeys = (pads: readonly (Gamepad | null)[]): number => {
  let keys = 0;
  for (const pad of pads) {
    if (!pad?.connected) continue;
    const down = (i: number) => pad.buttons[i]?.pressed ?? false;
    const [x = 0, y = 0] = pad.axes;

    if (down(1)) keys |= KEY.A;
    if (down(0)) keys |= KEY.B;
    if (down(3)) keys |= KEY.X;
    if (down(2)) keys |= KEY.Y;
    if (down(4) || down(6)) keys |= KEY.L;
    if (down(5) || down(7)) keys |= KEY.R;
    if (down(8)) keys |= KEY.SELECT;
    if (down(9)) keys |= KEY.START;
    // Web axes grow downwards / rightwards.
    if (down(12) || y < -STICK_THRESHOLD) keys |= KEY.UP;
    if (down(13) || y > STICK_THRESHOLD) keys |= KEY.DOWN;
    if (down(14) || x < -STICK_THRESHOLD) keys |= KEY.LEFT;
    if (down(15) || x > STICK_THRESHOLD) keys |= KEY.RIGHT;
  }
  return keys;
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

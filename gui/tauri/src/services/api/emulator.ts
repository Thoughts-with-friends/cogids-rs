import { invoke } from "@tauri-apps/api/core";

export type ScreenLayout = "natural" | "vertical" | "horizontal";
export type ScreenSizing =
  | "even"
  | "emphasizeTop"
  | "emphasizeBottom"
  | "topOnly"
  | "bottomOnly";

/** View menu settings (`ds_common::view::View`). */
export type View = {
  scale: number;
  layout: ScreenLayout;
  rotation: number;
  gap: number;
  swap: boolean;
  sizing: ScreenSizing;
  integerScaling: boolean;
  filtering: boolean;
  showFps: boolean;
};

/** GUI settings stored in `instances/instanceN/config.json`. */
export type GuiConfig = View & {
  romDir: string;
  selectedRom: string | null;
  /** 0-100 */
  volume: number;
  limitFramerate: boolean;
  recentRoms: string[];
};

export type EmuStatus = {
  running: boolean;
  paused: boolean;
  fps: number;
  /** File name of the running game. */
  rom: string | null;
  instanceDir: string;
  instance: number;
};

/** An auxiliary window opened from the menus (`ds_common::menu::Window`). */
export type DialogKind =
  | "romInfo"
  | "multiplayer"
  | "emuSettings"
  | "input"
  | "audio"
  | "wifi"
  | "paths"
  | "about";

/** A menu action (`ds_common::menu::Action`, serde `type`/`value`). */
export type Action =
  | { type: "openRom" }
  | { type: "openRomPath"; value: string }
  | { type: "clearRecent" }
  | { type: "eject" }
  | { type: "importSavefile" }
  | { type: "openDirectory" }
  | { type: "quit" }
  | { type: "togglePause" }
  | { type: "reset" }
  | { type: "stop" }
  | { type: "frameStep" }
  | { type: "launchInstance" }
  | { type: "screenSize"; value: number }
  | { type: "setRotation"; value: number }
  | { type: "setGap"; value: number }
  | { type: "setLayout"; value: ScreenLayout }
  | { type: "toggleSwap" }
  | { type: "setSizing"; value: ScreenSizing }
  | { type: "toggleIntegerScaling" }
  | { type: "toggleFiltering" }
  | { type: "toggleShowFps" }
  | { type: "toggleLimitFramerate" }
  | { type: "open"; value: DialogKind };

/** One menu entry (`ds_common::menu::Item`). */
export type MenuItem =
  | {
      kind: "entry";
      label: string;
      action: Action | null;
      enabled: boolean;
      checked: boolean | null;
      radio: boolean;
      reason: string | null;
      shortcut: string | null;
      hint: string | null;
    }
  | { kind: "submenu"; label: string; enabled: boolean; items: MenuItem[] }
  | { kind: "separator" }
  | { kind: "label"; text: string };

export type Menu = { label: string; items: MenuItem[] };

export type Rect = { x: number; y: number; w: number; h: number };
export type ScreenRect = { rect: Rect; rotation: number };
/** `ds_common::view::Placement`: `[top, bottom]`, `null` when hidden. */
export type Placement = {
  screens: [ScreenRect | null, ScreenRect | null];
  natural_w: number;
  natural_h: number;
};

export type Binary = {
  romOffset: number;
  entry: number;
  ramAddress: number;
  size: number;
};

export type RomInfo = {
  path: string;
  title: string;
  gameCode: string;
  makerCode: string;
  version: number;
  fileSize: number;
  capacity: number;
  arm9: Binary;
  arm7: Binary;
  titleJa: string | null;
  titleEn: string | null;
  /** 32x32 RGBA, empty without a banner. */
  iconRgba: number[];
};

/** `settings.json` (`ds_common::Settings`, snake_case). */
export type EmuSettings = {
  log_level: string;
  hle_bios: boolean;
  arm9_bios_path: string | null;
  arm7_bios_path: string | null;
  firmware_path: string | null;
  frameskip: number;
  mac_address: string | null;
};

/** Ctrl+letter shortcuts (`Action::from_shortcut`). */
export const SHORTCUTS: Record<string, Action> = {
  o: { type: "openRom" },
  p: { type: "togglePause" },
  r: { type: "reset" },
  n: { type: "frameStep" },
  q: { type: "quit" },
};

/**
 * The DS pixel under `(px, py)` (relative to the screen area) for a screen
 * drawn in `screen` (`ScreenRect::to_ds`); `clamp` keeps a drag that left
 * the screen on its edge.
 */
export const toDs = (
  screen: ScreenRect,
  px: number,
  py: number,
  clamp = false,
): { x: number; y: number } | null => {
  const r = screen.rect;
  let [x, y] = [px, py];
  if (clamp) {
    x = Math.min(Math.max(x, r.x), r.x + r.w - 0.01);
    y = Math.min(Math.max(y, r.y), r.y + r.h - 0.01);
  }
  if (x < r.x || y < r.y || x >= r.x + r.w || y >= r.y + r.h) return null;
  const [u, v] = [(x - r.x) / r.w, (y - r.y) / r.h];
  const [sx, sy] =
    screen.rotation === 90
      ? [v, 1 - u]
      : screen.rotation === 180
        ? [1 - u, 1 - v]
        : screen.rotation === 270
          ? [1 - v, u]
          : [u, v];
  return {
    x: Math.min(Math.max(Math.floor(sx * 256), 0), 255),
    y: Math.min(Math.max(Math.floor(sy * 192), 0), 191),
  };
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
  menu: () => invoke<Menu[]>("get_menu"),
  menuAction: (action: Action) => invoke<void>("menu_action", { action }),
  importSavefile: (path: string) => invoke<void>("import_savefile", { path }),
  romInfo: () => invoke<RomInfo>("rom_info"),
  getSettings: () => invoke<EmuSettings>("get_settings"),
  setSettings: (settings: EmuSettings) =>
    invoke<void>("set_settings", { settings }),
  layout: (width: number, height: number) =>
    invoke<Placement>("layout_screens", { width, height }),

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

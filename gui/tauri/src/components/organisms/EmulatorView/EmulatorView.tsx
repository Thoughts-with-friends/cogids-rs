import { Alert, Box, Button, Stack, Typography } from "@mui/material";
import { isTauri } from "@tauri-apps/api/core";
import { LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";
import type React from "react";
import { DsDialogs } from "@/components/organisms/DsDialogs/DsDialogs";
import { DsMenuBar } from "@/components/organisms/DsMenuBar/DsMenuBar";
import { useGamepads } from "@/components/hooks/useGamepads";
import { openPath } from "@/services/api/dialog";
import {
  type Action,
  type DialogKind,
  EMU,
  type EmuStatus,
  type GuiConfig,
  KEYMAP,
  type Menu,
  type Placement,
  SCREEN_H,
  SCREEN_W,
  SHORTCUTS,
  type ScreenRect,
  toDs,
} from "@/services/api/emulator";

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

const reportError = (error: unknown) => console.error(error);

/** CSS for a screen canvas placed by `layout_screens`. */
const screenStyle = (
  screen: ScreenRect | null,
  filtering: boolean,
): React.CSSProperties => {
  if (!screen) return { display: "none" };
  const { x, y, w, h } = screen.rect;
  // The canvas is laid out unrotated, centred on the box, then turned.
  const quarter = screen.rotation % 180 !== 0;
  const [cw, ch] = quarter ? [h, w] : [w, h];
  return {
    position: "absolute",
    left: x + (w - cw) / 2,
    top: y + (h - ch) / 2,
    width: cw,
    height: ch,
    aspectRatio: `${SCREEN_W} / ${SCREEN_H}`,
    transform: `rotate(${screen.rotation}deg)`,
    imageRendering: filtering ? "auto" : "pixelated",
    background: "#000",
  };
};

/**
 * Nintendo DS screens under melonDS's menu bar (File / System / View /
 * Config / Help, the same tree as the egui GUI).
 */
export const EmulatorView = () => {
  const [config, setConfig] = useState<GuiConfig | null>(null);
  const [menus, setMenus] = useState<Menu[]>([]);
  const [status, setStatus] = useState<EmuStatus | null>(null);
  const [dialog, setDialog] = useState<DialogKind | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [area, setArea] = useState({ width: 0, height: 0 });
  const [placement, setPlacement] = useState<Placement | null>(null);

  const areaRef = useRef<HTMLDivElement>(null);
  const upperRef = useRef<HTMLCanvasElement>(null);
  const lowerRef = useRef<HTMLCanvasElement>(null);
  /** Pressed DS buttons from the keyboard and from gamepads. */
  const keyboardKeysRef = useRef(0);
  const gamepadKeysRef = useRef(0);
  const touchingRef = useRef(false);

  // ===== Backend state =====

  /** Config, menu and status from the backend (after any action). */
  const refresh = useCallback(
    () =>
      Promise.all([EMU.getConfig(), EMU.menu(), EMU.status()])
        .then(([nextConfig, nextMenus, nextStatus]) => {
          setConfig(nextConfig);
          setMenus(nextMenus);
          setStatus(nextStatus);
        })
        .catch(reportError),
    [],
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Status (fps) twice a second; the menu follows game / pause changes.
  const statusKey = `${status?.rom}|${status?.paused}`;
  useEffect(() => {
    const id = window.setInterval(() => {
      EMU.status().then(setStatus).catch(reportError);
    }, 500);
    return () => window.clearInterval(id);
  }, []);
  useEffect(() => {
    EMU.menu().then(setMenus).catch(reportError);
  }, [statusKey]);

  useEffect(() => {
    if (!isTauri()) return;
    const title = status?.rom
      ? `${status.rom} - cogids (instance${status.instance})`
      : `cogids (instance${status?.instance ?? 1})`;
    getCurrentWindow().setTitle(title).catch(reportError);
  }, [status?.rom, status?.instance]);

  const updateConfig = useCallback((patch: Partial<GuiConfig>) => {
    setConfig((prev) => {
      if (!prev) return prev;
      const next = { ...prev, ...patch };
      EMU.setConfig(next)
        .then(() => EMU.menu().then(setMenus))
        .catch(reportError);
      return next;
    });
  }, []);

  // ===== Menu actions =====

  const romDir = config?.romDir ?? "";
  const handleAction = useCallback(
    async (action: Action) => {
      setError(null);
      try {
        switch (action.type) {
          case "openRom": {
            const path = await openPath(romDir, {
              filters: [{ name: "Nintendo DS ROM", extensions: ["nds"] }],
            });
            if (typeof path === "string") {
              await EMU.menuAction({ type: "openRomPath", value: path });
            }
            break;
          }
          case "importSavefile": {
            const path = await openPath("", {
              filters: [{ name: "DS save", extensions: ["sav", "dsv", "bin"] }],
            });
            if (typeof path === "string") await EMU.importSavefile(path);
            break;
          }
          case "quit":
            // CloseRequested stops the game and writes its save.
            await getCurrentWindow().close();
            return;
          case "open":
            setDialog(action.value);
            return;
          case "screenSize": {
            await EMU.menuAction(action);
            const box = areaRef.current?.getBoundingClientRect();
            const natural = await EMU.layout(1, 1);
            if (box && isTauri()) {
              // Keep whatever the page puts around the screens.
              const chromeW = window.innerWidth - box.width;
              const chromeH = window.innerHeight - box.height;
              await getCurrentWindow().setSize(
                new LogicalSize(
                  natural.natural_w * action.value + chromeW,
                  natural.natural_h * action.value + chromeH,
                ),
              );
            }
            break;
          }
          default:
            await EMU.menuAction(action);
        }
      } catch (e) {
        setError(String(e));
      }
      await refresh();
    },
    [romDir, refresh],
  );

  // ===== Screen placement =====

  // The area exists once the config has loaded (before, "Loading...").
  const loaded = config !== null;
  useEffect(() => {
    const el = areaRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setArea({ width, height });
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [loaded]);

  useEffect(() => {
    if (!config || area.width === 0) return;
    EMU.layout(area.width, area.height).then(setPlacement).catch(reportError);
  }, [
    area,
    config,
    config?.rotation,
    config?.layout,
    config?.gap,
    config?.swap,
    config?.sizing,
    config?.integerScaling,
  ]);

  // ===== Frame rendering =====

  useEffect(() => {
    let alive = true;
    let pending = false;
    let lastSeq = 0;
    let animationId = 0;

    const tick = () => {
      if (!alive) return;
      const upper = upperRef.current?.getContext("2d");
      const lower = lowerRef.current?.getContext("2d");

      if (!pending && upper && lower) {
        pending = true;
        EMU.frame(lastSeq)
          .then((frame) => {
            if (!alive || !frame) return;
            lastSeq = frame.seq;
            upper.putImageData(new ImageData(frame.upper, SCREEN_W, SCREEN_H), 0, 0);
            lower.putImageData(new ImageData(frame.lower, SCREEN_W, SCREEN_H), 0, 0);
          })
          .catch(reportError)
          .finally(() => {
            pending = false;
          });
      }
      animationId = requestAnimationFrame(tick);
    };

    animationId = requestAnimationFrame(tick);
    return () => {
      alive = false;
      cancelAnimationFrame(animationId);
    };
  }, []);

  // ===== Input (keyboard + gamepads + shortcuts) =====

  const sendKeys = useCallback(() => {
    EMU.setKeys(keyboardKeysRef.current | gamepadKeysRef.current).catch(reportError);
  }, []);

  const gamepads = useGamepads((keys) => {
    gamepadKeysRef.current = keys;
    sendKeys();
  });

  const handleKey = useCallback(
    (pressed: boolean) => (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      // Do not intercept text input.
      if (
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable
      ) {
        return;
      }

      if (pressed && (e.ctrlKey || e.metaKey) && !e.repeat) {
        const action = SHORTCUTS[e.key.toLowerCase()];
        if (action) {
          e.preventDefault();
          void handleAction(action);
          return;
        }
      }

      const bit = KEYMAP[e.code];
      if (bit === undefined) return;
      e.preventDefault();

      const next = pressed ? keyboardKeysRef.current | bit : keyboardKeysRef.current & ~bit;
      if (next === keyboardKeysRef.current) return;
      keyboardKeysRef.current = next;
      sendKeys();
    },
    [handleAction, sendKeys],
  );

  const releaseAllKeys = useCallback(() => {
    keyboardKeysRef.current = 0;
    sendKeys();
  }, [sendKeys]);

  useEffect(() => {
    const down = handleKey(true);
    const up = handleKey(false);
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", releaseAllKeys);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", releaseAllKeys);
    };
  }, [handleKey, releaseAllKeys]);

  // ===== Touch screen =====

  const touchAt = useCallback(
    (e: React.PointerEvent<HTMLDivElement>, clamp: boolean) => {
      const bottom = placement?.screens[1];
      if (!bottom) return null;
      const box = e.currentTarget.getBoundingClientRect();
      return toDs(bottom, e.clientX - box.left, e.clientY - box.top, clamp);
    },
    [placement],
  );

  const handlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const pos = touchAt(e, false);
      if (!pos) return; // only a press on the bottom screen starts a touch
      e.currentTarget.setPointerCapture(e.pointerId);
      touchingRef.current = true;
      EMU.setTouch(pos).catch(reportError);
    },
    [touchAt],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (!touchingRef.current) return;
      const pos = touchAt(e, true);
      if (pos) EMU.setTouch(pos).catch(reportError);
    },
    [touchAt],
  );

  const handlePointerUp = useCallback(() => {
    if (!touchingRef.current) return;
    touchingRef.current = false;
    EMU.setTouch(null).catch(reportError);
  }, []);

  // ===== Render =====

  if (!config) {
    return <Typography>Loading...</Typography>;
  }

  const running = !!status?.running;
  const welcomeRoms = config.recentRoms.slice(0, 10);

  return (
    <Box sx={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <DsMenuBar menus={menus} onAction={(action) => void handleAction(action)} />

      <Box
        ref={areaRef}
        data-test="screen-area"
        sx={{
          position: "relative",
          flex: 1,
          minHeight: 0,
          overflow: "hidden",
          bgcolor: "#000",
          touchAction: "none",
          cursor: running ? "crosshair" : "default",
        }}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerUp}
      >
        {/* Native canvases: the bitmap must be exactly 256x192 (the
            on-screen size is CSS only). */}
        <canvas
          ref={upperRef}
          width={SCREEN_W}
          height={SCREEN_H}
          style={running ? screenStyle(placement?.screens[0] ?? null, config.filtering) : { display: "none", aspectRatio: `${SCREEN_W} / ${SCREEN_H}` }}
        />
        <canvas
          ref={lowerRef}
          width={SCREEN_W}
          height={SCREEN_H}
          style={running ? screenStyle(placement?.screens[1] ?? null, config.filtering) : { display: "none", aspectRatio: `${SCREEN_W} / ${SCREEN_H}` }}
        />

        {running && config.showFps && (
          <Typography
            variant="caption"
            sx={{
              position: "absolute",
              left: 6,
              top: 4,
              color: "#8fe08f",
              fontFamily: "monospace",
              pointerEvents: "none",
            }}
          >
            {status?.fps.toFixed(1)} fps{status?.paused ? "  (paused)" : ""}
          </Typography>
        )}

        {!running && (
          <Stack spacing={1} alignItems="center" sx={{ pt: 4, color: "#ddd" }}>
            <Typography variant="h6">No game running</Typography>
            <Typography variant="body2">
              File &gt; Open ROM... (Ctrl+O), Open recent or ROM folder
            </Typography>
            {welcomeRoms.map((rom) => (
              <Button
                key={rom}
                size="small"
                variant="outlined"
                title={rom}
                onClick={() => void handleAction({ type: "openRomPath", value: rom })}
              >
                {fileName(rom)}
              </Button>
            ))}
          </Stack>
        )}
      </Box>

      {error && (
        <Alert severity="error" onClose={() => setError(null)} sx={{ borderRadius: 0 }}>
          {error}
        </Alert>
      )}

      <DsDialogs
        open={dialog}
        onClose={() => setDialog(null)}
        config={config}
        updateConfig={updateConfig}
        status={status}
        gamepads={gamepads}
      />
    </Box>
  );
};

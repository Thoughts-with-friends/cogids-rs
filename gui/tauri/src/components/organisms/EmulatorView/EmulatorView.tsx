import FolderOpenIcon from "@mui/icons-material/FolderOpen";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import StopIcon from "@mui/icons-material/Stop";
import {
  Box,
  Button,
  FormControlLabel,
  IconButton,
  MenuItem,
  Radio,
  RadioGroup,
  Slider,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useRef, useState } from "react";
import type React from "react";
import { useGamepads } from "@/components/hooks/useGamepads";
import { openPath } from "@/services/api/dialog";
import {
  EMU,
  type EmuStatus,
  type GuiConfig,
  KEYMAP,
  SCREEN_H,
  SCREEN_W,
} from "@/services/api/emulator";

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

const reportError = (error: unknown) => console.error(error);

/** Nintendo DS screens, ROM selection and emulator controls. */
export const EmulatorView = () => {
  const [config, setConfig] = useState<GuiConfig | null>(null);
  const [roms, setRoms] = useState<string[]>([]);
  const [status, setStatus] = useState<EmuStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const upperRef = useRef<HTMLCanvasElement>(null);
  const lowerRef = useRef<HTMLCanvasElement>(null);
  /** Pressed DS buttons from the keyboard and from gamepads. */
  const keyboardKeysRef = useRef(0);
  const gamepadKeysRef = useRef(0);
  const touchingRef = useRef(false);

  // ===== Config =====

  useEffect(() => {
    const loadConfig = async () => {
      try {
        setConfig(await EMU.getConfig());
      } catch (e) {
        reportError(e);
      }
    };

    void loadConfig();
  }, []);

  const updateConfig = useCallback((patch: Partial<GuiConfig>) => {
    setConfig((prev) => {
      if (!prev) return prev;

      const next = { ...prev, ...patch };
      EMU.setConfig(next).catch(reportError);
      return next;
    });
  }, []);

  const loadRoms = useCallback(async (romDir: string) => {
    try {
      setRoms(await EMU.listRoms(romDir));
    } catch (e) {
      reportError(e);
    }
  }, []);

  useEffect(() => {
    if (!config?.romDir) {
      setRoms([]);
      return;
    }

    void loadRoms(config.romDir);
  }, [config?.romDir, loadRoms]);

  // ===== Emulator status =====

  const fetchStatus = useCallback(async () => {
    try {
      setStatus(await EMU.status());
    } catch (e) {
      reportError(e);
    }
  }, []);

  useEffect(() => {
    void fetchStatus();

    const id = window.setInterval(() => {
      void fetchStatus();
    }, 500);

    return () => window.clearInterval(id);
  }, [fetchStatus]);

  // ===== Frame rendering =====

  const drawFrame = useCallback(() => {
    const upper = upperRef.current?.getContext("2d");
    const lower = lowerRef.current?.getContext("2d");

    return { upper, lower };
  }, []);

  useEffect(() => {
    let alive = true;
    let pending = false;
    let lastSeq = 0;
    let animationId = 0;

    const tick = () => {
      if (!alive) return;

      const { upper, lower } = drawFrame();

      if (!pending && upper && lower) {
        pending = true;

        EMU.frame(lastSeq)
          .then((frame) => {
            if (!alive || !frame) return;

            lastSeq = frame.seq;

            upper.putImageData(
              new ImageData(frame.upper, SCREEN_W, SCREEN_H),
              0,
              0,
            );

            lower.putImageData(
              new ImageData(frame.lower, SCREEN_W, SCREEN_H),
              0,
              0,
            );
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
  }, [drawFrame]);

  // ===== Input (keyboard + gamepads) =====

  const sendKeys = useCallback(() => {
    EMU.setKeys(keyboardKeysRef.current | gamepadKeysRef.current).catch(
      reportError,
    );
  }, []);

  const gamepads = useGamepads((keys) => {
    gamepadKeysRef.current = keys;
    sendKeys();
  });


  const handleKey = useCallback(
    (pressed: boolean) => (e: KeyboardEvent) => {
      const bit = KEYMAP[e.code];
      const target = e.target as HTMLElement | null;

      // Do not intercept text input.
      if (
        bit === undefined ||
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable
      ) {
        return;
      }

      e.preventDefault();

      const next = pressed
        ? keyboardKeysRef.current | bit
        : keyboardKeysRef.current & ~bit;

      if (next === keyboardKeysRef.current) return;

      keyboardKeysRef.current = next;
      sendKeys();
    },
    [sendKeys],
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
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      const rect = e.currentTarget.getBoundingClientRect();

      const x = Math.floor(
        ((e.clientX - rect.left) / rect.width) * SCREEN_W,
      );
      const y = Math.floor(
        ((e.clientY - rect.top) / rect.height) * SCREEN_H,
      );

      EMU.setTouch({ x, y }).catch(reportError);
    },
    [],
  );

  const handlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      e.currentTarget.setPointerCapture(e.pointerId);
      touchingRef.current = true;
      touchAt(e);
    },
    [touchAt],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      if (touchingRef.current) touchAt(e);
    },
    [touchAt],
  );

  const handlePointerUp = useCallback(() => {
    touchingRef.current = false;
    EMU.setTouch(null).catch(reportError);
  }, []);

  // ===== Controls =====

  const browseRomDirectory = useCallback(async () => {
    try {
      const dir = await openPath(config?.romDir ?? "", {
        directory: true,
      });

      if (typeof dir === "string") {
        updateConfig({ romDir: dir });
      }
    } catch (e) {
      setError(String(e));
    }
  }, [config?.romDir, updateConfig]);

  const startEmulator = useCallback(async () => {
    if (!config?.selectedRom) return;

    setError(null);

    try {
      await EMU.start(config.selectedRom);
    } catch (e) {
      setError(String(e));
    }
  }, [config?.selectedRom]);

  const stopEmulator = useCallback(async () => {
    try {
      await EMU.stop();
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // ===== UI helpers =====

  const renderRomControls = () => (
    <>
      <Stack direction="row" spacing={1} alignItems="center">
        <TextField
          label="ROM folder"
          size="small"
          fullWidth
          value={config!.romDir}
          onChange={(e) => updateConfig({ romDir: e.target.value })}
        />
        <IconButton onClick={browseRomDirectory} title="Browse">
          <FolderOpenIcon />
        </IconButton>
      </Stack>

      <Stack
        direction="row"
        spacing={1}
        alignItems="center"
        flexWrap="wrap"
        useFlexGap
      >
        <TextField
          select
          label="ROM"
          size="small"
          sx={{ minWidth: 240, flex: "1 1 240px" }}
          value={config!.selectedRom ?? ""}
          onChange={(e) => updateConfig({ selectedRom: e.target.value })}
        >
          {roms.map((rom) => (
            <MenuItem key={rom} value={rom}>
              {fileName(rom)}
            </MenuItem>
          ))}
        </TextField>

        <Button
          variant="contained"
          startIcon={<PlayArrowIcon />}
          onClick={startEmulator}
          disabled={!config!.selectedRom}
        >
          Run
        </Button>

        <Button
          variant="outlined"
          startIcon={<StopIcon />}
          onClick={stopEmulator}
        >
          Stop
        </Button>
      </Stack>
    </>
  );

  const renderSettings = () => (
    <Stack
      direction="row"
      spacing={3}
      alignItems="center"
      flexWrap="wrap"
      useFlexGap
    >
      <Box sx={{ width: 200 }}>
        <Typography variant="caption">
          Scale: {config!.scale.toFixed(1)}x
        </Typography>
        <Slider
          size="small"
          min={1}
          max={5}
          step={0.1}
          value={config!.scale}
          onChange={(_, value) =>
            updateConfig({ scale: value as number })
          }
        />
      </Box>

      <Box sx={{ width: 200 }}>
        <Typography variant="caption">
          Volume: {config!.volume}%
        </Typography>
        <Slider
          size="small"
          min={0}
          max={100}
          value={config!.volume}
          onChange={(_, value) =>
            updateConfig({ volume: value as number })
          }
        />
      </Box>

      <RadioGroup
        row
        value={config!.layout}
        onChange={(e) =>
          updateConfig({
            layout: e.target.value as GuiConfig["layout"],
          })
        }
      >
        <FormControlLabel
          value="vertical"
          control={<Radio size="small" />}
          label="Vertical"
        />
        <FormControlLabel
          value="horizontal"
          control={<Radio size="small" />}
          label="Horizontal"
        />
      </RadioGroup>

      <Typography variant="body2" color="text.secondary">
        {status?.running ? `${status.fps.toFixed(1)} fps` : "Stopped"}
      </Typography>
    </Stack>
  );

  const renderScreens = () => {
    // Native <canvas> elements: the bitmap must be exactly 256x192. (MUI
    // `<Box component="canvas" width height>` treats width/height as CSS,
    // leaving the default 300x150 bitmap: the frame was cropped and
    // stretched vertically.) The on-screen size is CSS only.
    const canvasStyle = {
      width: `min(${SCREEN_W * config!.scale}px, 100%)`,
      height: "auto",
      aspectRatio: `${SCREEN_W} / ${SCREEN_H}`,
      imageRendering: "pixelated",
      background: "#000",
      display: "block",
      minWidth: 0,
    } as const satisfies React.CSSProperties;

    return (
      <Stack
        direction={config!.layout === "horizontal" ? "row" : "column"}
        spacing={1}
        alignItems="center"
        justifyContent="center"
      >
        <canvas
          ref={upperRef}
          width={SCREEN_W}
          height={SCREEN_H}
          style={canvasStyle}
        />

        <canvas
          ref={lowerRef}
          width={SCREEN_W}
          height={SCREEN_H}
          style={{
            ...canvasStyle,
            cursor: "crosshair",
            touchAction: "none",
          }}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={handlePointerUp}
          onPointerCancel={handlePointerUp}
        />
      </Stack>
    );
  };

  const renderControllerInfo = () => (
    <Typography variant="caption" color="text.secondary">
      Controller:{" "}
      {gamepads.length
        ? gamepads.join(", ")
        : "none (press a button to detect one; keyboard: arrows, X=A, Z=B, S=X, A=Y, Q=L, W=R, Enter=START, Backspace=SELECT)"}
      {status && ` | Data: ${status.instanceDir}`}
    </Typography>
  );

  // ===== Main render =====

  if (!config) {
    return <Typography>Loading...</Typography>;
  }

  return (
    <Stack spacing={2} sx={{ width: "100%", maxWidth: 1400, minWidth: 0 }}>
      {renderRomControls()}
      {renderSettings()}

      {error && <Typography color="error">{error}</Typography>}

      {renderScreens()}
      {renderControllerInfo()}
    </Stack>
  );
};
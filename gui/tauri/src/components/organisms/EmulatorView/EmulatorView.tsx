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

/** Nintendo DS screens, ROM selection and emulator controls. */
export const EmulatorView = () => {
  const [config, setConfigState] = useState<GuiConfig | null>(null);
  const [roms, setRoms] = useState<string[]>([]);
  const [status, setStatus] = useState<EmuStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const upperRef = useRef<HTMLCanvasElement>(null);
  const lowerRef = useRef<HTMLCanvasElement>(null);
  const keysRef = useRef(0);
  const touchingRef = useRef(false);

  // ===== config =====
  useEffect(() => {
    EMU.getConfig().then(setConfigState).catch(console.error);
  }, []);

  const updateConfig = useCallback((patch: Partial<GuiConfig>) => {
    setConfigState((prev) => {
      if (!prev) return prev;
      const next = { ...prev, ...patch };
      EMU.setConfig(next).catch(console.error);
      return next;
    });
  }, []);

  useEffect(() => {
    if (config?.romDir) {
      EMU.listRoms(config.romDir).then(setRoms).catch(console.error);
    }
  }, [config?.romDir]);

  // ===== status (running / fps / controllers) =====
  useEffect(() => {
    const id = setInterval(() => {
      EMU.status().then(setStatus).catch(console.error);
    }, 500);
    return () => clearInterval(id);
  }, []);

  // ===== frames =====
  useEffect(() => {
    let alive = true;
    let lastSeq = 0;
    let pending = false;

    const tick = () => {
      if (!alive) return;
      // Look the canvases up every frame: they only exist once the config
      // has loaded (the first render shows "Loading...").
      const upper = upperRef.current?.getContext("2d");
      const lower = lowerRef.current?.getContext("2d");
      if (!pending && upper && lower) {
        pending = true;
        EMU.frame(lastSeq)
          .then((frame) => {
            if (!frame) return;
            lastSeq = frame.seq;
            upper.putImageData(new ImageData(frame.upper, SCREEN_W, SCREEN_H), 0, 0);
            lower.putImageData(new ImageData(frame.lower, SCREEN_W, SCREEN_H), 0, 0);
          })
          .catch(console.error)
          .finally(() => {
            pending = false;
          });
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    return () => {
      alive = false;
    };
  }, []);

  // ===== keyboard =====
  useEffect(() => {
    const onKey = (pressed: boolean) => (e: KeyboardEvent) => {
      const bit = KEYMAP[e.code];
      // Leave text fields alone.
      if (bit === undefined || (e.target as HTMLElement)?.tagName === "INPUT") return;
      e.preventDefault();
      const next = pressed ? keysRef.current | bit : keysRef.current & ~bit;
      if (next !== keysRef.current) {
        keysRef.current = next;
        EMU.setKeys(next).catch(console.error);
      }
    };
    const down = onKey(true);
    const up = onKey(false);
    const blur = () => {
      keysRef.current = 0;
      EMU.setKeys(0).catch(console.error);
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
    };
  }, []);

  // ===== touch screen =====
  const touchAt = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const x = Math.floor(((e.clientX - rect.left) / rect.width) * SCREEN_W);
    const y = Math.floor(((e.clientY - rect.top) / rect.height) * SCREEN_H);
    EMU.setTouch({ x, y }).catch(console.error);
  };
  const onPointerDown = (e: React.PointerEvent<HTMLCanvasElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    touchingRef.current = true;
    touchAt(e);
  };
  const onPointerMove = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (touchingRef.current) touchAt(e);
  };
  const onPointerUp = () => {
    touchingRef.current = false;
    EMU.setTouch(null).catch(console.error);
  };

  // ===== controls =====
  const browse = async () => {
    const dir = await openPath(config?.romDir ?? "", { directory: true });
    if (typeof dir === "string") updateConfig({ romDir: dir });
  };

  const run = async () => {
    if (!config?.selectedRom) return;
    setError(null);
    try {
      await EMU.start(config.selectedRom);
    } catch (e) {
      setError(String(e));
    }
  };

  if (!config) {
    return <Typography>Loading...</Typography>;
  }

  const width = SCREEN_W * config.scale;
  const height = SCREEN_H * config.scale;
  const canvasSx = {
    width,
    height,
    imageRendering: "pixelated",
    background: "#000",
    display: "block",
  } as const;

  return (
    <Stack spacing={2} sx={{ width: "100%", maxWidth: 1400, minWidth: 0 }}>
      <Stack direction="row" spacing={1} alignItems="center">
        <TextField
          label="ROM folder"
          size="small"
          fullWidth
          value={config.romDir}
          onChange={(e) => updateConfig({ romDir: e.target.value })}
        />
        <IconButton onClick={browse} title="Browse">
          <FolderOpenIcon />
        </IconButton>
      </Stack>

      <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
        <TextField
          select
          label="ROM"
          size="small"
          sx={{ minWidth: 240, flex: "1 1 240px" }}
          value={config.selectedRom ?? ""}
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
          onClick={run}
          disabled={!config.selectedRom}
        >
          Run
        </Button>
        <Button variant="outlined" startIcon={<StopIcon />} onClick={() => EMU.stop()}>
          Stop
        </Button>
      </Stack>

      <Stack direction="row" spacing={3} alignItems="center" flexWrap="wrap" useFlexGap>
        <Box sx={{ width: 200 }}>
          <Typography variant="caption">Scale: {config.scale.toFixed(1)}x</Typography>
          <Slider
            size="small"
            min={1}
            max={5}
            step={0.1}
            value={config.scale}
            onChange={(_, v) => updateConfig({ scale: v as number })}
          />
        </Box>
        <Box sx={{ width: 200 }}>
          <Typography variant="caption">Volume: {config.volume}%</Typography>
          <Slider
            size="small"
            min={0}
            max={100}
            value={config.volume}
            onChange={(_, v) => updateConfig({ volume: v as number })}
          />
        </Box>
        <RadioGroup
          row
          value={config.layout}
          onChange={(e) => updateConfig({ layout: e.target.value as GuiConfig["layout"] })}
        >
          <FormControlLabel value="vertical" control={<Radio size="small" />} label="Vertical" />
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

      {error && <Typography color="error">{error}</Typography>}

      <Stack
        direction={config.layout === "horizontal" ? "row" : "column"}
        spacing={1}
        alignItems="center"
        justifyContent="center"
      >
        <Box component="canvas" ref={upperRef} width={SCREEN_W} height={SCREEN_H} sx={canvasSx} />
        <Box
          component="canvas"
          ref={lowerRef}
          width={SCREEN_W}
          height={SCREEN_H}
          sx={{ ...canvasSx, cursor: "crosshair", touchAction: "none" }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        />
      </Stack>

      <Typography variant="caption" color="text.secondary">
        Controller:{" "}
        {status?.gamepads.length
          ? status.gamepads.join(", ")
          : "none (keyboard: arrows, X=A, Z=B, S=X, A=Y, Q=L, W=R, Enter=START, Backspace=SELECT)"}
        {status && ` | Data: ${status.instanceDir}`}
      </Typography>
    </Stack>
  );
};

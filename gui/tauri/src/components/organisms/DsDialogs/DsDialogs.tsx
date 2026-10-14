import {
  Alert,
  Box,
  Button,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  MenuItem,
  Slider,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { openPath } from "@/services/api/dialog";
import {
  type DialogKind,
  EMU,
  type EmuSettings,
  type EmuStatus,
  type GuiConfig,
  type RomInfo,
} from "@/services/api/emulator";

type Props = Readonly<{
  open: DialogKind | null;
  onClose: () => void;
  config: GuiConfig;
  updateConfig: (patch: Partial<GuiConfig>) => void;
  status: EmuStatus | null;
  gamepads: string[];
}>;

const TITLES: Record<DialogKind, string> = {
  romInfo: "ROM info",
  multiplayer: "Multiplayer",
  emuSettings: "Emu settings",
  input: "Input and hotkeys",
  audio: "Audio settings",
  wifi: "Wifi settings",
  paths: "Path settings",
  about: "About",
};

const MAC_RE = /^([0-9a-f]{2}[:-]){5}[0-9a-f]{2}$/i;

/** The windows the menus open (melonDS's dialogs). */
export const DsDialogs = (props: Props) => {
  const { open, onClose } = props;
  if (!open) return null;

  return (
    <Dialog open onClose={onClose} maxWidth="md">
      <DialogTitle>{TITLES[open]}</DialogTitle>
      <DialogContent dividers>
        {open === "romInfo" && <RomInfoView />}
        {open === "emuSettings" && <SettingsView kind="emu" />}
        {open === "wifi" && <SettingsView kind="wifi" status={props.status} />}
        {open === "input" && <InputView gamepads={props.gamepads} />}
        {open === "audio" && (
          <AudioView config={props.config} updateConfig={props.updateConfig} />
        )}
        {open === "paths" && (
          <PathsView
            config={props.config}
            updateConfig={props.updateConfig}
            status={props.status}
          />
        )}
        {open === "about" && <AboutView />}
        {open === "multiplayer" && (
          <Typography>LAN play is only available in the egui GUI for now.</Typography>
        )}
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Close</Button>
      </DialogActions>
    </Dialog>
  );
};

const hex = (n: number) => n.toString(16).toUpperCase().padStart(8, "0");

const RomInfoView = () => {
  const [info, setInfo] = useState<RomInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const iconRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    EMU.romInfo().then(setInfo).catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    const ctx = iconRef.current?.getContext("2d");
    if (!ctx || !info || info.iconRgba.length !== 32 * 32 * 4) return;
    ctx.putImageData(new ImageData(new Uint8ClampedArray(info.iconRgba), 32, 32), 0, 0);
  }, [info]);

  if (error) return <Alert severity="error">{error}</Alert>;
  if (!info) return <Typography>Loading...</Typography>;

  const rows: [string, string][] = [
    ["Title", info.title],
    ["Game code", info.gameCode],
    ["Maker code", info.makerCode],
    ["Version", String(info.version)],
    ["File size", `${info.fileSize} bytes`],
    ["Capacity", `${info.capacity / (1024 * 1024)} MiB`],
    ...(["arm9", "arm7"] as const).map(
      (cpu): [string, string] => [
        cpu.toUpperCase(),
        `ROM ${hex(info[cpu].romOffset)}  entry ${hex(info[cpu].entry)}  RAM ${hex(info[cpu].ramAddress)}  size ${hex(info[cpu].size)}`,
      ],
    ),
    ["Path", info.path],
  ];

  return (
    <Stack spacing={2}>
      <Stack direction="row" spacing={2} alignItems="center">
        {info.iconRgba.length > 0 && (
          <canvas
            ref={iconRef}
            width={32}
            height={32}
            style={{ width: 64, height: 64, imageRendering: "pixelated" }}
          />
        )}
        <Box>
          {[info.titleJa, info.titleEn]
            .filter((t): t is string => !!t)
            .map((t) => (
              <Typography key={t} sx={{ whiteSpace: "pre-line" }}>
                {t}
              </Typography>
            ))}
        </Box>
      </Stack>
      <Table size="small">
        <TableBody>
          {rows.map(([k, v]) => (
            <TableRow key={k}>
              <TableCell>{k}</TableCell>
              <TableCell sx={{ fontFamily: "monospace" }}>{v}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </Stack>
  );
};

type PathKey = "arm9_bios_path" | "arm7_bios_path" | "firmware_path";

/** Emu settings and Wifi settings: both edit `settings.json`. */
const SettingsView = ({
  kind,
  status,
}: Readonly<{ kind: "emu" | "wifi"; status?: EmuStatus | null }>) => {
  const [settings, setSettings] = useState<EmuSettings | null>(null);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    EMU.getSettings()
      .then(setSettings)
      .catch((e) => setMessage({ ok: false, text: String(e) }));
  }, []);

  if (!settings) return <Typography>Loading...</Typography>;

  const save = async (next: EmuSettings) => {
    try {
      await EMU.setSettings(next);
      setSettings(next);
      setMessage({ ok: true, text: "Saved. Applies when a game is next started or reset." });
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    }
  };

  const pathRow = (label: string, key: PathKey) => (
    <Stack direction="row" spacing={1} alignItems="center" key={key}>
      <TextField
        label={label}
        size="small"
        sx={{ minWidth: 420 }}
        placeholder="(built-in)"
        value={settings[key] ?? ""}
        onChange={(e) => setSettings({ ...settings, [key]: e.target.value || null })}
      />
      <Button
        onClick={async () => {
          const picked = await openPath(settings[key] ?? "");
          if (typeof picked === "string") setSettings({ ...settings, [key]: picked });
        }}
      >
        Browse...
      </Button>
    </Stack>
  );

  return (
    <Stack spacing={2} sx={{ pt: 1 }}>
      {kind === "emu" ? (
        <>
          <FormControlLabel
            control={
              <Checkbox
                checked={settings.hle_bios}
                onChange={(e) => setSettings({ ...settings, hle_bios: e.target.checked })}
              />
            }
            label="High-level BIOS emulation (no BIOS dump needed)"
          />
          {pathRow("ARM9 BIOS", "arm9_bios_path")}
          {pathRow("ARM7 BIOS", "arm7_bios_path")}
          {pathRow("Firmware", "firmware_path")}
          <Stack direction="row" spacing={2}>
            <TextField
              label="Frame skip"
              type="number"
              size="small"
              value={settings.frameskip}
              slotProps={{ htmlInput: { min: 0, max: 9 } }}
              onChange={(e) =>
                setSettings({ ...settings, frameskip: Number(e.target.value) || 0 })
              }
            />
            <TextField
              select
              label="Log level"
              size="small"
              sx={{ minWidth: 140 }}
              value={settings.log_level}
              onChange={(e) => setSettings({ ...settings, log_level: e.target.value })}
            >
              {["error", "warn", "info", "debug", "trace"].map((l) => (
                <MenuItem key={l} value={l}>
                  {l}
                </MenuItem>
              ))}
            </TextField>
          </Stack>
        </>
      ) : (
        <>
          <Typography>
            Instance: instance{status?.instance ?? "?"} (each instance keeps its own MAC)
          </Typography>
          <Stack direction="row" spacing={1} alignItems="center">
            <TextField
              label="MAC address"
              size="small"
              value={settings.mac_address ?? ""}
              error={!MAC_RE.test(settings.mac_address ?? "")}
              helperText="format: 00:09:BF:12:34:56"
              onChange={(e) => setSettings({ ...settings, mac_address: e.target.value })}
            />
            <Button
              onClick={() => {
                const bytes = Array.from({ length: 3 }, () =>
                  Math.floor(Math.random() * 256)
                    .toString(16)
                    .toUpperCase()
                    .padStart(2, "0"),
                );
                setSettings({ ...settings, mac_address: ["00", "09", "BF", ...bytes].join(":") });
              }}
            >
              Randomize
            </Button>
          </Stack>
          <Typography variant="caption" color="text.secondary">
            Consoles playing together need different MACs.
          </Typography>
        </>
      )}
      {message && <Alert severity={message.ok ? "success" : "error"}>{message.text}</Alert>}
      <Box>
        <Button variant="contained" onClick={() => void save(settings)}>
          Save
        </Button>
      </Box>
    </Stack>
  );
};

const KEY_ROWS: [string, string, string][] = [
  ["A", "X", "right face button"],
  ["B", "Z", "bottom face button"],
  ["X", "S", "top face button"],
  ["Y", "A", "left face button"],
  ["L / R", "Q / W", "shoulders / triggers"],
  ["START / SELECT", "Enter / Backspace", "Start / Back"],
  ["D-pad", "Arrow keys", "D-pad / left stick"],
  ["Touch screen", "Mouse (left button)", "-"],
];

const HOTKEYS: [string, string][] = [
  ["Ctrl+O", "Open ROM"],
  ["Ctrl+P", "Pause / resume"],
  ["Ctrl+R", "Reset"],
  ["Ctrl+N", "Frame step"],
  ["Ctrl+Q", "Quit"],
];

const InputView = ({ gamepads }: Readonly<{ gamepads: string[] }>) => (
  <Stack spacing={2}>
    <Table size="small">
      <TableHead>
        <TableRow>
          <TableCell>DS</TableCell>
          <TableCell>Keyboard</TableCell>
          <TableCell>Gamepad</TableCell>
        </TableRow>
      </TableHead>
      <TableBody>
        {KEY_ROWS.map(([ds, key, pad]) => (
          <TableRow key={ds}>
            <TableCell>{ds}</TableCell>
            <TableCell sx={{ fontFamily: "monospace" }}>{key}</TableCell>
            <TableCell>{pad}</TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
    <Typography variant="subtitle2">Hotkeys</Typography>
    {HOTKEYS.map(([key, what]) => (
      <Typography key={key} variant="body2">
        <code>{key}</code> {what}
      </Typography>
    ))}
    <Typography variant="body2">
      Gamepads: {gamepads.length ? gamepads.join(", ") : "none (press a button to detect one)"}
    </Typography>
    <Typography variant="caption" color="text.secondary">
      The mapping is fixed; remapping is not implemented yet.
    </Typography>
  </Stack>
);

const AudioView = ({
  config,
  updateConfig,
}: Readonly<{ config: GuiConfig; updateConfig: (p: Partial<GuiConfig>) => void }>) => (
  <Box sx={{ width: 320 }}>
    <Typography variant="caption">Volume: {config.volume}%</Typography>
    <Slider
      min={0}
      max={100}
      value={config.volume}
      onChange={(_, value) => updateConfig({ volume: value as number })}
    />
  </Box>
);

const PathsView = ({
  config,
  updateConfig,
  status,
}: Readonly<{
  config: GuiConfig;
  updateConfig: (p: Partial<GuiConfig>) => void;
  status: EmuStatus | null;
}>) => (
  <Stack spacing={2} sx={{ pt: 1 }}>
    <Stack direction="row" spacing={1} alignItems="center">
      <TextField
        label="ROM folder"
        size="small"
        sx={{ minWidth: 420 }}
        value={config.romDir}
        onChange={(e) => updateConfig({ romDir: e.target.value })}
      />
      <Button
        onClick={async () => {
          const dir = await openPath(config.romDir, { directory: true });
          if (typeof dir === "string") updateConfig({ romDir: dir });
        }}
      >
        Browse...
      </Button>
    </Stack>
    <Typography variant="body2">Saves, settings and logs (this instance):</Typography>
    <Stack direction="row" spacing={1} alignItems="center">
      <Typography sx={{ fontFamily: "monospace" }}>{status?.instanceDir}</Typography>
      <Button onClick={() => void EMU.menuAction({ type: "openDirectory" })}>Open</Button>
    </Stack>
  </Stack>
);

const AboutView = () => (
  <Stack spacing={1}>
    <Typography variant="h6">cogids-rs</Typography>
    <Typography>A Nintendo DS emulator written in Rust.</Typography>
    <Typography>
      Core based on CorgiDS; Wi-Fi and multiplayer after melonDS (melonds_net). Menus laid out
      after melonDS.
    </Typography>
    <Typography>License: GPL-3.0</Typography>
    <Typography>github.com/Thoughts-with-friends/cogids-rs</Typography>
  </Stack>
);

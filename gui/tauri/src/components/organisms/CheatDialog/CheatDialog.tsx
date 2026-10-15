import {
  Alert,
  Box,
  Button,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Paper,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { type Cheat, EMU, type RomInfo } from "@/services/api/emulator";
import { cheatCodeError } from "./cheatCode";

type Props = Readonly<{ onClose: () => void }>;

const ACTION_REPLAY = "Action Replay";

const humanSize = (bytes: number) => {
  const mib = bytes / (1024 * 1024);
  return mib >= 1024 ? `${(mib / 1024).toFixed(1)} GiB` : `${mib.toFixed(1)} MiB`;
};

type Draft = { name: string; memo: string; code: string };
const EMPTY: Draft = { name: "", memo: "", code: "" };
const draftOf = (c: Cheat | undefined): Draft =>
  c ? { name: c.name, memo: c.memo, code: c.code } : EMPTY;

/**
 * System > Setup cheat codes: the game on the left, its cheats (enabled,
 * name, type) in the middle and the selected cheat's name / memo / code on
 * the right (`docs/design/cheat_gui.png`). OK saves the list and runs the
 * enabled codes; Cancel forgets the changes.
 */
export const CheatDialog = ({ onClose }: Props) => {
  const [list, setList] = useState<Cheat[] | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [draft, setDraft] = useState<Draft>(EMPTY);
  const [info, setInfo] = useState<RomInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const iconRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    EMU.getCheats()
      .then((cheats) => {
        setList(cheats);
        if (cheats.length > 0) {
          setSelected(0);
          setDraft(draftOf(cheats[0]));
        }
      })
      .catch((e) => setError(String(e)));
    EMU.romInfo().then(setInfo).catch(() => setInfo(null));
  }, []);

  useEffect(() => {
    const ctx = iconRef.current?.getContext("2d");
    if (!ctx || !info || info.iconRgba.length !== 32 * 32 * 4) return;
    ctx.putImageData(new ImageData(new Uint8ClampedArray(info.iconRgba), 32, 32), 0, 0);
  }, [info]);

  const cheats = list ?? [];
  const current = selected === null ? undefined : cheats[selected];
  const codeError = current ? cheatCodeError(draft.code) : null;
  const edited =
    !!current &&
    (current.name !== draft.name || current.memo !== draft.memo || current.code !== draft.code);

  /** The list with the editor's changes in the selected cheat (if valid). */
  const withDraft = (): Cheat[] =>
    edited && !codeError && selected !== null
      ? cheats.map((c, i) => (i === selected ? { ...c, ...draft } : c))
      : cheats;

  const select = (next: Cheat[], index: number | null) => {
    setList(next);
    setSelected(index);
    setDraft(draftOf(index === null ? undefined : next[index]));
  };

  const add = () => {
    const next = [
      ...withDraft(),
      { name: "New cheat", memo: "", code: "", enabled: false, kind: ACTION_REPLAY },
    ];
    select(next, next.length - 1);
  };

  const remove = () => {
    if (selected === null) return;
    const next = cheats.filter((_, i) => i !== selected);
    select(next, next.length === 0 ? null : Math.min(selected, next.length - 1));
  };

  const toggle = (index: number) =>
    setList(cheats.map((c, i) => (i === index ? { ...c, enabled: !c.enabled } : c)));

  const ok = () => {
    EMU.setCheats(withDraft())
      .then(onClose)
      .catch((e) => setError(String(e)));
  };

  const infoRows: [string, string][] = info
    ? [
        ["Name", info.titleJa?.split("\n")[0] ?? info.titleEn?.split("\n")[0] ?? info.title],
        ["Game code", info.gameCode],
        ["Format", "NDS"],
        ["Size", humanSize(info.fileSize)],
        ["Filepath", info.path],
      ]
    : [];

  return (
    <Dialog open onClose={onClose} maxWidth="lg" fullWidth>
      <DialogTitle>Cheats</DialogTitle>
      <DialogContent dividers>
        <Stack direction="row" spacing={2} sx={{ minHeight: 480 }}>
          {/* Info */}
          <Paper variant="outlined" sx={{ width: 240, p: 1.5, flexShrink: 0 }}>
            <Typography align="center" variant="subtitle2" sx={{ mb: 1 }}>
              Info
            </Typography>
            {info && info.iconRgba.length > 0 && (
              <Box sx={{ display: "flex", justifyContent: "center", mb: 1 }}>
                <canvas
                  ref={iconRef}
                  width={32}
                  height={32}
                  style={{ width: 96, height: 96, imageRendering: "pixelated" }}
                />
              </Box>
            )}
            <Stack spacing={1}>
              {infoRows.map(([label, value]) => (
                <TextField
                  key={label}
                  label={label}
                  value={value}
                  size="small"
                  title={value}
                  slotProps={{ input: { readOnly: true } }}
                />
              ))}
            </Stack>
          </Paper>

          {/* List and editor */}
          <Stack spacing={1} sx={{ flex: 1, minWidth: 0 }}>
            <Stack direction="row" justifyContent="flex-end">
              <Button variant="outlined" onClick={add} disabled={list === null}>
                Add cheat
              </Button>
            </Stack>
            <Stack direction="row" alignItems="center" spacing={1}>
              <Typography sx={{ flex: 1 }}>Available cheats:</Typography>
              <Button
                variant="outlined"
                disabled={!edited || !!codeError}
                onClick={() => select(withDraft(), selected)}
              >
                Save
              </Button>
              <Button variant="outlined" disabled={selected === null} onClick={remove}>
                Delete
              </Button>
            </Stack>
            <Stack direction="row" spacing={2} sx={{ flex: 1, minHeight: 0 }}>
              <TableContainer component={Paper} variant="outlined" sx={{ flex: 1, maxHeight: 420 }}>
                <Table size="small" stickyHeader>
                  <TableHead>
                    <TableRow>
                      <TableCell padding="checkbox" />
                      <TableCell>Name</TableCell>
                      <TableCell>Type</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {cheats.map((cheat, i) => (
                      <TableRow
                        key={i}
                        hover
                        selected={i === selected}
                        onClick={() => i !== selected && select(withDraft(), i)}
                        sx={{ cursor: "pointer" }}
                        data-test="cheat-row"
                      >
                        <TableCell padding="checkbox">
                          <Checkbox
                            size="small"
                            checked={cheat.enabled}
                            onClick={(e) => e.stopPropagation()}
                            onChange={() => toggle(i)}
                            slotProps={{ input: { "aria-label": `Enable ${cheat.name}` } }}
                          />
                        </TableCell>
                        <TableCell
                          sx={{
                            maxWidth: 260,
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                          }}
                        >
                          {cheat.name}
                        </TableCell>
                        <TableCell>{cheat.kind}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </TableContainer>

              <Stack spacing={1.5} sx={{ flex: 1, minWidth: 0 }}>
                <TextField
                  label="Name"
                  size="small"
                  value={draft.name}
                  disabled={!current}
                  onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                />
                <TextField
                  label="Memo"
                  multiline
                  minRows={7}
                  maxRows={7}
                  value={draft.memo}
                  disabled={!current}
                  onChange={(e) => setDraft({ ...draft, memo: e.target.value })}
                />
                <TextField
                  label="Code"
                  multiline
                  minRows={6}
                  maxRows={6}
                  value={draft.code}
                  disabled={!current}
                  placeholder="02000000 00000063"
                  error={!!codeError}
                  helperText={codeError ?? " "}
                  slotProps={{ htmlInput: { style: { fontFamily: "monospace" } } }}
                  onChange={(e) => setDraft({ ...draft, code: e.target.value })}
                />
              </Stack>
            </Stack>
          </Stack>
        </Stack>
        {error && (
          <Alert severity="error" sx={{ mt: 1 }}>
            {error}
          </Alert>
        )}
      </DialogContent>
      <DialogActions>
        <Button variant="contained" onClick={ok} disabled={list === null}>
          OK
        </Button>
        <Button onClick={onClose}>Cancel</Button>
      </DialogActions>
    </Dialog>
  );
};

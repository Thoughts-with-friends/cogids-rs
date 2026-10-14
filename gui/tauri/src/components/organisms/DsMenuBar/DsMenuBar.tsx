import CheckIcon from "@mui/icons-material/Check";
import ChevronRightIcon from "@mui/icons-material/ChevronRight";
import RadioButtonCheckedIcon from "@mui/icons-material/RadioButtonChecked";
import RadioButtonUncheckedIcon from "@mui/icons-material/RadioButtonUnchecked";
import {
  Box,
  Button,
  Divider,
  ListItemIcon,
  ListItemText,
  Menu as MuiMenu,
  MenuItem,
  Tooltip,
  Typography,
} from "@mui/material";
import { useState } from "react";
import type { Action, Menu, MenuItem as Item } from "@/services/api/emulator";

type Props = Readonly<{
  menus: Menu[];
  onAction: (action: Action) => void;
}>;

/**
 * melonDS-style menu bar drawn from the backend's menu tree
 * (`ds_common::menu`, the same one the egui GUI draws). Disabled entries
 * explain themselves on hover.
 */
export const DsMenuBar = ({ menus, onAction }: Props) => {
  const [open, setOpen] = useState<{ index: number; anchor: HTMLElement } | null>(
    null,
  );

  const pick = (action: Action) => {
    setOpen(null);
    onAction(action);
  };

  return (
    <Box
      component="nav"
      sx={{
        display: "flex",
        gap: 0.5,
        px: 0.5,
        borderBottom: 1,
        borderColor: "divider",
        bgcolor: "background.paper",
      }}
    >
      {menus.map((menu, index) => (
        <Button
          key={menu.label}
          size="small"
          color="inherit"
          sx={{ textTransform: "none", minWidth: 0 }}
          onClick={(e) => setOpen({ index, anchor: e.currentTarget })}
          // Like a desktop menu bar: with one menu open, hovering another
          // switches to it.
          onMouseEnter={(e) => {
            if (open && open.index !== index) {
              setOpen({ index, anchor: e.currentTarget });
            }
          }}
        >
          {menu.label}
        </Button>
      ))}

      {open && menus[open.index] && (
        <MuiMenu
          open
          anchorEl={open.anchor}
          onClose={() => setOpen(null)}
          slotProps={{ list: { dense: true } }}
        >
          <Items items={menus[open.index].items} onPick={pick} />
        </MuiMenu>
      )}
    </Box>
  );
};

type ItemsProps = Readonly<{
  items: Item[];
  onPick: (action: Action) => void;
}>;

/** The entries of one (sub)menu; at most one submenu open at a time. */
const Items = ({ items, onPick }: ItemsProps) => {
  const [sub, setSub] = useState<{ index: number; anchor: HTMLElement } | null>(
    null,
  );

  return items.map((item, index) => {
    switch (item.kind) {
      case "separator":
        return <Divider key={`sep-${index}`} />;

      case "label":
        return (
          <Typography
            key={`label-${index}`}
            variant="caption"
            color="text.secondary"
            sx={{ display: "block", px: 2, py: 0.5 }}
          >
            {item.text}
          </Typography>
        );

      case "submenu":
        return (
          <Box key={`sub-${item.label}`}>
            <MenuItem
              disabled={!item.enabled}
              onMouseEnter={(e) => setSub({ index, anchor: e.currentTarget })}
              onClick={(e) => setSub({ index, anchor: e.currentTarget })}
            >
              <ListItemText inset>{item.label}</ListItemText>
              <ChevronRightIcon fontSize="small" />
            </MenuItem>
            {sub?.index === index && (
              <MuiMenu
                open
                anchorEl={sub.anchor}
                onClose={() => setSub(null)}
                anchorOrigin={{ vertical: "top", horizontal: "right" }}
                transformOrigin={{ vertical: "top", horizontal: "left" }}
                slotProps={{ list: { dense: true } }}
              >
                <Items items={item.items} onPick={onPick} />
              </MuiMenu>
            )}
          </Box>
        );

      case "entry": {
        const icon =
          item.checked === null ? null : item.radio ? (
            item.checked ? (
              <RadioButtonCheckedIcon fontSize="small" />
            ) : (
              <RadioButtonUncheckedIcon fontSize="small" />
            )
          ) : item.checked ? (
            <CheckIcon fontSize="small" />
          ) : null;
        const action = item.action;
        const entry = (
          <MenuItem
            disabled={!item.enabled}
            onMouseEnter={() => setSub(null)}
            onClick={() => action && onPick(action)}
          >
            {icon ? <ListItemIcon>{icon}</ListItemIcon> : null}
            <ListItemText inset={!icon}>{item.label}</ListItemText>
            {item.shortcut && (
              <Typography variant="body2" color="text.secondary" sx={{ ml: 3 }}>
                {item.shortcut}
              </Typography>
            )}
          </MenuItem>
        );
        const tip = item.reason ?? item.hint;
        // A disabled element gets no hover events: the span does.
        return tip ? (
          <Tooltip key={`entry-${index}`} title={tip} placement="right">
            <span>{entry}</span>
          </Tooltip>
        ) : (
          <Box key={`entry-${index}`}>{entry}</Box>
        );
      }
    }
  });
};

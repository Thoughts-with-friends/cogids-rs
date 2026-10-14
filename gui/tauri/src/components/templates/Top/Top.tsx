"use client";

import { Box, type SxProps, type Theme } from "@mui/material";
import { useInjectJs } from "@/components/hooks/useInjectJs";
import { EmulatorView } from "@/components/organisms/EmulatorView";
import { showWindow } from "@/services/api/window";

export const Top = () => {
  useInjectJs();
  showWindow();

  return (
    <Box component="main" sx={pageSx}>
      <EmulatorView />
    </Box>
  );
};

const pageSx: SxProps<Theme> = {
  // The screens fill the window above the bottom navigation (56px).
  height: "calc(100vh - 56px)",
  width: "100%",
  overflow: "hidden",
  boxSizing: "border-box",
};

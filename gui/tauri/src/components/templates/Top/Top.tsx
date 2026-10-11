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
  display: "flex",
  justifyContent: "center",
  overflowX: "hidden",
  minHeight: "calc(100vh - 56px)",
  pt: 2,
  pb: 10,
  px: 2,
  width: "100%",
  boxSizing: "border-box",
};

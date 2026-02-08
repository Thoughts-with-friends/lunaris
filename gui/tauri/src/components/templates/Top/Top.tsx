"use client";

import { Box, type SxProps, type Theme } from "@mui/material";
import { useInjectJs } from "@/components/hooks/useInjectJs";
import { WelcomePage } from "./WelcomePage/WelcomePage";
import { showWindow } from "@/services/api/window";

export const Top = () => {
  useInjectJs();
  showWindow();

  return (
    <Box component="main" sx={pageSx}>
      <WelcomePage />
    </Box>
  );
};

const pageSx: SxProps<Theme> = {
  display: "grid",
  minHeight: "calc(100vh - 56px)",
  placeItems: "center",
  pt: 8,
  px: 2,
  width: "100%",
};

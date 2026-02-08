import { type SxProps, type Theme } from "@mui/material";

export const pageSx: SxProps<Theme> = {
  display: "grid",
  minHeight: "calc(100vh - 56px)",
  placeItems: "center",
  pt: 8,
  px: 2,
  width: "100%",
};

export const toolCardSx: SxProps<Theme> = (theme) => ({
  bgcolor:
    theme.palette.mode === "dark"
      ? "rgba(30, 30, 30, 0.6)"
      : "rgba(255, 255, 255, 0.7)",
  backdropFilter: "blur(5px)",
  borderRadius: 4,
  transition: "transform 0.2s, box-shadow 0.2s",
  "&:hover": {
    transform: "translateY(-4px)",
    boxShadow: 8,
  },
});

export const toolCardContentSx: SxProps<Theme> = {
  textAlign: "center",
  p: 4,
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  "&:hover": {
    color: "var(--mui-palette-primary-main)",
  },
};

export const titleSx: SxProps<Theme> = {
  mt: 2,
};

export const descSx: SxProps<Theme> = {
  mt: 1,
};

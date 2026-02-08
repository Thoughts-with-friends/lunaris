import { Grid, Typography } from "@mui/material";
import { useTranslation } from "@/components/hooks/useTranslation";
import { MenuBar } from "../MenuBar/MenuBar";
import { ToolCard } from "./ToolCard";
import { useToolCardItem } from "./config";

export const WelcomePage = () => {
  const { t } = useTranslation();
  const tools = useToolCardItem();

  return (
    <>
      <MenuBar />

      <Typography
        variant="h3"
        sx={{ mb: 1, fontWeight: "bold", letterSpacing: 1 }}
      >
        {t("welcome.title")}
      </Typography>
      <Typography variant="subtitle1" sx={{ mb: 6, color: "text.secondary" }}>
        {t("welcome.subtitle")}
      </Typography>

      <Grid
        container
        spacing={4}
        justifyContent="center"
        sx={{ maxWidth: 900 }}
      >
        {tools.map(({ key, ...tool }) => (
          <Grid key={key} size={{ xs: 12, sm: 6, md: 6 }}>
            <ToolCard {...tool} />
          </Grid>
        ))}
      </Grid>
    </>
  );
};

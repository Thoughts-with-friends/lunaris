import LayersIcon from "@mui/icons-material/Layers";
import TransformIcon from "@mui/icons-material/Transform";
import { useTranslation } from "@/components/hooks/useTranslation";
import type { ToolCardItem } from "./types";
import { NOTIFY } from "@/lib/notify";
import { useRouter } from "@tanstack/react-router";

export /** Define the start menu config */
const useToolCardItem = () => {
  const { t } = useTranslation();
  const router = useRouter();

  return [
    {
      key: "open",
      icon: <TransformIcon sx={{ fontSize: 48 }} />,
      title: t("welcome.tools.open.title"),
      description: t("welcome.tools.open.desc"),
      onClick: () => {
        NOTIFY.info("Clicked open.");
      },
    },
    {
      key: "latest",
      icon: <LayersIcon sx={{ fontSize: 48 }} />,
      title: t("welcome.tools.latest.title"),
      description: t("welcome.tools.latest.desc"),
      onClick: () => {
        NOTIFY.info("Clicked latest.");
      },
    },
    {
      key: "recent",
      icon: <LayersIcon sx={{ fontSize: 48 }} />,
      title: t("welcome.tools.recent.title"),
      description: t("welcome.tools.recent.desc"),
      onClick: () => {
        NOTIFY.info("Clicked recent.");
      },
    },
    {
      key: "settings",
      icon: <LayersIcon sx={{ fontSize: 48 }} />,
      title: t("welcome.tools.settings.title"),
      description: t("welcome.tools.settings.desc"),
      onClick: () => {
        router.navigate({ to: "/settings" });
      },
    },
  ] as const satisfies ToolCardItem[];
};

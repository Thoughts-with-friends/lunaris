import type { DrawerMenuItem } from "./DrawerList";
import InboxIcon from "@mui/icons-material/MoveToInbox";
import MailIcon from "@mui/icons-material/Mail";
import { NOTIFY } from "@/lib/notify";
import { useTranslation } from "@/components/hooks/useTranslation";
import { destroyCurrentWindow } from "@/services/api/window";

export const useDrawerMenu = () => {
  const { t } = useTranslation();

  return [
    {
      label: "File",
      icon: <InboxIcon />,
      children: [
        {
          label: t("menubar.file.submenu.open.label"),
          onClick: () => NOTIFY.success("Open ROM clicked."),
        },
        {
          label: t("menubar.file.submenu.recent.label"),
          onClick: () => NOTIFY.success("Recent ROM clicked."),
        },
        {
          label: t("menubar.file.submenu.quit.label"),
          onClick: async () => await destroyCurrentWindow(),
        },
      ],
    },
    {
      label: "Edit",
      icon: <MailIcon />,
      children: [{ label: "Undo" }, { label: "Redo" }],
    },
  ] as const satisfies DrawerMenuItem[];
};

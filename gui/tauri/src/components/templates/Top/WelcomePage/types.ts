import type { ReactNode } from "react";

export type ToolCardItem = {
  key: string;
  icon: ReactNode;
  title: string;
  description: string;
  onClick: () => void;
};

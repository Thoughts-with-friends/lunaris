import { Card, CardActionArea, CardContent, Typography } from "@mui/material";
import { toolCardSx, toolCardContentSx, titleSx, descSx } from "./styles";
import type { ToolCardItem } from "./types";

type Props = Omit<ToolCardItem, "key">;

export const ToolCard = ({ icon, title, description, onClick }: Props) => {
  return (
    <Card elevation={4} sx={toolCardSx}>
      <CardActionArea onClick={onClick} sx={{ height: "100%" }}>
        <CardContent sx={toolCardContentSx}>
          {icon}
          <Typography variant="h6" sx={titleSx}>
            {title}
          </Typography>
          <Typography variant="body2" sx={descSx}>
            {description}
          </Typography>
        </CardContent>
      </CardActionArea>
    </Card>
  );
};

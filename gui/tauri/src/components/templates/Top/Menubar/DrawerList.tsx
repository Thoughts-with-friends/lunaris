import { useState } from "react";
import List from "@mui/material/List";
import ListItem from "@mui/material/ListItem";
import ListItemButton from "@mui/material/ListItemButton";
import ListItemIcon from "@mui/material/ListItemIcon";
import ListItemText from "@mui/material/ListItemText";
import Menu from "@mui/material/Menu";
import MenuItem from "@mui/material/MenuItem";
import ChevronRightIcon from "@mui/icons-material/ChevronRight";

export interface DrawerMenuItem {
  label: string;
  icon?: React.ReactNode;
  onClick?: () => void;
  children?: DrawerMenuItem[];
}

interface Props {
  open: boolean;
  items: DrawerMenuItem[];
}

// Main export
export function DrawerList({ open, items }: Props) {
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null);
  const [activeItem, setActiveItem] = useState<DrawerMenuItem | null>(null);

  // Drawer item
  const handleEnterItem = ((e, item) => {
    if (!item.children?.length) return;
    setAnchorEl(e.currentTarget);
    setActiveItem(item);
  }) satisfies ItemMenuProps["handleEnterItem"];

  // All handle leaving
  const handleLeaveAll = () => {
    setAnchorEl(null);
    setActiveItem(null);
  };

  return (
    <>
      <List>
        {items.map((item) => (
          <ItemMenu item={item} open={open} handleEnterItem={handleEnterItem} />
        ))}
      </List>

      <SubMenu
        anchorEl={anchorEl}
        activeItem={activeItem}
        handleLeaveAll={handleLeaveAll}
      />
    </>
  );
}

// Item Menu component
type ItemMenuProps = {
  item: DrawerMenuItem;
  open: boolean;
  handleEnterItem: (
    e: React.MouseEvent<HTMLElement>,
    item: DrawerMenuItem,
  ) => void;
};

const ItemMenu = ({ item, open, handleEnterItem }: ItemMenuProps) => {
  return (
    <ListItem
      key={item.label}
      disablePadding
      sx={{ display: "block" }}
      onMouseEnter={(e) => handleEnterItem(e, item)}
    >
      <ListItemButton
        sx={{
          minHeight: 48,
          px: 2.5,
          justifyContent: open ? "initial" : "center",
        }}
      >
        {item.icon && (
          <ListItemIcon
            sx={{
              minWidth: 0,
              mr: open ? 3 : "auto",
              justifyContent: "center",
            }}
          >
            {item.icon}
          </ListItemIcon>
        )}

        <ListItemText primary={item.label} sx={{ opacity: open ? 1 : 0 }} />

        {item.children && open && <ChevronRightIcon />}
      </ListItemButton>
    </ListItem>
  );
};

// Sub Menu component
type SubMenuProps = {
  anchorEl: HTMLElement | null;
  activeItem: DrawerMenuItem | null;
  handleLeaveAll: () => void;
};

const SubMenu = ({ anchorEl, activeItem, handleLeaveAll }: SubMenuProps) => {
  return (
    <Menu
      anchorEl={anchorEl}
      open={Boolean(anchorEl)}
      onClose={handleLeaveAll}
      anchorOrigin={{
        vertical: "top",
        horizontal: "right",
      }}
      transformOrigin={{
        vertical: "top",
        horizontal: "left",
      }}
      MenuListProps={{
        onMouseLeave: handleLeaveAll,
      }}
      disableAutoFocusItem
    >
      {activeItem?.children?.map((child) => (
        <MenuItem
          key={child.label}
          onClick={() => {
            child.onClick?.();
            handleLeaveAll();
          }}
        >
          {child.label}
        </MenuItem>
      ))}
    </Menu>
  );
};

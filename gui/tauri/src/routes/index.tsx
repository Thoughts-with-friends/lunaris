import { Top } from "@/components/templates/Top";
import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/")({
  component: Top,
});

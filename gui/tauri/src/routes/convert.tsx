import { Convert } from "@/components/templates/Convert";
import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/convert")({
  component: Convert,
});

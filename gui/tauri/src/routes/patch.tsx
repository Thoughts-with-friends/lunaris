import { Patch } from "@/components/templates/Patch";
import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/patch")({
  component: Patch,
});

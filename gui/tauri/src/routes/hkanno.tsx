import { HkannoEditorPage } from "@/components/templates/HkAnno";
import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/hkanno")({
  component: HkannoEditorPage,
});

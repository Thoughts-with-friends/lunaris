import ClientLayout from "@/components/layout/ClientLayout/ClientLayout";
import { Outlet, createRootRoute } from "@tanstack/react-router";

export const Route = createRootRoute({
  component: RootComponent,
});

function RootComponent() {
  return (
    <>
      <ClientLayout>
        <Outlet />
      </ClientLayout>
    </>
  );
}

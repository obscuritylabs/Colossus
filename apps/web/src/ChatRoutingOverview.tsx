import { lazy, Suspense, type ComponentProps } from "react";
import { ModelRoutingOverview } from "@colossus/ui/components/ModelRoutingOverview";

const ModelRoutingGraph = lazy(() =>
  import("@colossus/ui/components/ModelRoutingGraph").then((module) => ({
    default: module.ModelRoutingGraph,
  })),
);

export function ChatRoutingOverview({
  instanceScope,
  ...props
}: ComponentProps<typeof ModelRoutingOverview> & { instanceScope: boolean }) {
  if (!instanceScope) return <ModelRoutingOverview {...props} />;
  return (
    <section className="model-routing-overview" aria-label="Routing map">
      <Suspense fallback={<p role="status">Loading routing map…</p>}>
        <ModelRoutingGraph {...props} />
      </Suspense>
    </section>
  );
}

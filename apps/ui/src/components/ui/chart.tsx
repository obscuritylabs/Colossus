// Adapted from the MIT shadcn radix-nova chart registry; Recharts v3 composition.
// https://ui.shadcn.com/docs/components/radix/chart
import {
  createContext,
  useContext,
  useId,
  type CSSProperties,
  type HTMLAttributes,
  type ReactElement,
  type ReactNode,
} from "react";
import {
  ResponsiveContainer,
  Tooltip,
  Legend,
  type TooltipContentProps,
  type DefaultLegendContentProps,
} from "recharts";
export interface ChartSeries {
  label: ReactNode;
  color: `var(--${string})`;
  dashed?: boolean;
}
export type ChartConfig = Record<string, ChartSeries>;
const ChartContext = createContext<ChartConfig | null>(null);
function useChart() {
  const config = useContext(ChartContext);
  if (!config) throw new Error("Chart content requires ChartContainer.");
  return config;
}
export function ChartContainer({
  config,
  children,
  className = "",
  style,
  ...props
}: HTMLAttributes<HTMLDivElement> & {
  config: ChartConfig;
  children: ReactElement;
}) {
  const id = useId(),
    colors = Object.fromEntries(
      Object.entries(config).map(([key, value]) => [
        `--color-${key}`,
        value.color,
      ]),
    );
  return (
    <ChartContext value={config}>
      <div
        {...props}
        data-slot="chart"
        data-chart={id}
        className={`shared-chart-container ${className}`}
        style={{ ...colors, ...style } as CSSProperties}
      >
        <ResponsiveContainer
          width="100%"
          height="100%"
          initialDimension={{ width: 640, height: 260 }}
        >
          {children}
        </ResponsiveContainer>
      </div>
    </ChartContext>
  );
}
export const ChartTooltip = Tooltip;
export const ChartLegend = Legend;
export function ChartTooltipContent({
  active,
  payload,
  label,
  labelFormatter,
}: Partial<TooltipContentProps<number, string>> & {
  labelFormatter?: (value: string) => ReactNode;
}) {
  const config = useChart();
  if (!active || !payload?.length) return null;
  return (
    <div className="shared-chart-tooltip">
      <strong>
        {labelFormatter ? labelFormatter(String(label)) : String(label)}
      </strong>
      <div>
        {payload
          .filter(
            (item) =>
              item.type !== "none" &&
              item.value !== undefined &&
              item.value !== null,
          )
          .map((item) => {
            const key = String(item.dataKey ?? item.name),
              series = config[key];
            return (
              <div className="shared-chart-tooltip-row" key={key}>
                <span
                  className={`shared-chart-indicator ${series?.dashed ? "is-dashed" : ""}`}
                  style={{ borderColor: series?.color ?? item.color }}
                />
                <span>{series?.label ?? item.name}</span>
                <strong>
                  {typeof item.value === "number"
                    ? item.value.toLocaleString()
                    : String(item.value)}
                </strong>
              </div>
            );
          })}
      </div>
    </div>
  );
}
export function ChartLegendContent({
  payload,
}: Partial<DefaultLegendContentProps>) {
  const config = useChart();
  return (
    <div className="shared-chart-legend">
      {payload
        ?.filter((item) => item.type !== "none")
        .map((item) => {
          const key = String(item.dataKey ?? item.value),
            series = config[key];
          return (
            <span key={key}>
              <i
                className={`shared-chart-indicator ${series?.dashed ? "is-dashed" : ""}`}
                style={{ borderColor: series?.color ?? item.color }}
              />
              {series?.label ?? item.value}
            </span>
          );
        })}
    </div>
  );
}

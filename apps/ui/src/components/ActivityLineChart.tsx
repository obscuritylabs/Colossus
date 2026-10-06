import { useId } from "react";
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  ChartLegend,
  ChartLegendContent,
  type ChartConfig,
} from "./ui/chart.js";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table.js";
export interface ActivityPoint {
  date: string;
  runs: number | null;
  completed: number | null;
  failed: number | null;
}
const config = {
  runs: { label: "Total runs", color: "var(--blue-strong)" },
  completed: { label: "Completed", color: "var(--green)", dashed: true },
  failed: { label: "Failed", color: "var(--red)", dashed: true },
} satisfies ChartConfig;
const shortDate = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
  timeZone: "UTC",
});
const longDate = new Intl.DateTimeFormat(undefined, {
  month: "long",
  day: "numeric",
  year: "numeric",
  timeZone: "UTC",
});
export function activityDate(value: string, full = false) {
  const date = new Date(`${value}T00:00:00Z`);
  return Number.isNaN(date.getTime())
    ? value
    : (full ? longDate : shortDate).format(date);
}
/** Display only supplied daily outcomes. Unknown values stay gaps; no synthetic days or interpolation. */
export function ActivityLineChart({
  data,
  title = "Daily run activity",
}: {
  data: ActivityPoint[];
  title?: string;
}) {
  const tableId = useId();
  if (!data.length)
    return (
      <p className="shared-chart-empty">
        No released run activity in this period.
      </p>
    );
  return (
    <div className="shared-activity-plot">
      <ChartContainer config={config} aria-label={title} role="group">
        <LineChart
          accessibilityLayer
          data={data}
          margin={{ top: 12, right: 12, bottom: 4, left: 4 }}
          title={`${title}. Daily UTC buckets; press arrow keys to inspect values.`}
        >
          <CartesianGrid
            vertical={false}
            stroke="var(--border)"
            strokeDasharray="3 3"
          />
          <XAxis
            dataKey="date"
            tickLine={false}
            axisLine={false}
            minTickGap={32}
            tickMargin={12}
            tickFormatter={(value) => activityDate(String(value))}
          />
          <YAxis
            tickLine={false}
            axisLine={false}
            allowDecimals={false}
            width={40}
            tickMargin={8}
            domain={[0, "auto"]}
          />
          <ChartTooltip
            cursor={{ stroke: "var(--border-strong)", strokeDasharray: "3 3" }}
            content={
              <ChartTooltipContent
                labelFormatter={(value) => activityDate(String(value), true)}
              />
            }
          />
          <ChartLegend content={<ChartLegendContent />} />
          <Line
            type="linear"
            dataKey="runs"
            name="Total runs"
            stroke="var(--color-runs)"
            strokeWidth={2.5}
            dot={data.length <= 7 ? { r: 3, strokeWidth: 0 } : false}
            activeDot={{ r: 4, strokeWidth: 2, stroke: "var(--surface)" }}
            connectNulls={false}
            isAnimationActive={false}
          />
          <Line
            type="linear"
            dataKey="completed"
            name="Completed"
            stroke="var(--color-completed)"
            strokeWidth={2}
            strokeDasharray="6 3"
            dot={false}
            activeDot={{ r: 4 }}
            connectNulls={false}
            isAnimationActive={false}
          />
          <Line
            type="linear"
            dataKey="failed"
            name="Failed"
            stroke="var(--color-failed)"
            strokeWidth={2}
            strokeDasharray="2 3"
            dot={false}
            activeDot={{ r: 4 }}
            connectNulls={false}
            isAnimationActive={false}
          />
        </LineChart>
      </ChartContainer>
      <div className="shared-chart-footer">
        <span>Daily execution outcomes · UTC</span>
        <details className="shared-chart-data">
          <summary aria-controls={tableId}>View data table</summary>
          <div className="shared-chart-table" id={tableId}>
            <Table>
              <caption>{title} · supplied daily UTC outcomes</caption>
              <TableHeader>
                <TableRow>
                  <TableHead scope="col">Date</TableHead>
                  <TableHead scope="col">Total runs</TableHead>
                  <TableHead scope="col">Completed</TableHead>
                  <TableHead scope="col">Failed</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.map((day) => (
                  <TableRow key={day.date}>
                    <TableHead scope="row">
                      <time dateTime={day.date}>
                        {activityDate(day.date, true)}
                      </time>
                    </TableHead>
                    <TableCell>{day.runs ?? "Unavailable"}</TableCell>
                    <TableCell>{day.completed ?? "Unavailable"}</TableCell>
                    <TableCell>{day.failed ?? "Unavailable"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </details>
      </div>
    </div>
  );
}

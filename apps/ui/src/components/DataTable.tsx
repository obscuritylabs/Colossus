import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  columnFilteringFeature,
  columnVisibilityFeature,
  rowPaginationFeature,
  rowSortingFeature,
  createFilteredRowModel,
  createPaginatedRowModel,
  createSortedRowModel,
  createColumnHelper,
  filterFn_includesString,
  sortFn_alphanumeric,
  tableFeatures,
  useTable,
  type ColumnFiltersState,
  type ColumnVisibilityState,
  type PaginationState,
  type SortingState,
} from "@tanstack/react-table";
import {
  IconArrowDown,
  IconArrowUp,
  IconArrowsSort,
  IconChevronLeft,
  IconChevronRight,
  IconColumns3,
  IconSearch,
} from "@tabler/icons-react";
import { Button, TextInput } from "./Controls.js";
import { DropdownSelect } from "./DropdownSelect.js";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table.js";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu.js";

const features = /* @__PURE__ */ tableFeatures({
  columnFilteringFeature,
  columnVisibilityFeature,
  rowPaginationFeature,
  rowSortingFeature,
  filteredRowModel: /* @__PURE__ */ createFilteredRowModel(),
  paginatedRowModel: /* @__PURE__ */ createPaginatedRowModel(),
  sortedRowModel: /* @__PURE__ */ createSortedRowModel(),
  filterFns: { includesString: filterFn_includesString },
  sortFns: { alphanumeric: sortFn_alphanumeric },
});

export interface DataTableColumn<T> {
  id: string;
  label: string;
  value: (row: T) => string | number;
  cell?: (row: T) => ReactNode;
  sortable?: boolean;
  hideable?: boolean;
  rowHeader?: boolean;
  className?: string;
  filter?: {
    label: string;
    options: { value: string; label: string }[];
    matches: (row: T, value: string) => boolean;
  };
}
export interface DataTableProps<T> {
  data: T[];
  columns: DataTableColumn<T>[];
  getRowId: (row: T) => string;
  label: string;
  itemLabel?: string;
  search?: { columnId: string; label: string };
  initialSorting?: SortingState;
  empty?: ReactNode;
  loading?: boolean;
  footer?: ReactNode;
}

/** A presentation-only table. Hosts provide data, permissions, and action callbacks. */
export function DataTable<T extends object>({
  data,
  columns,
  getRowId,
  label,
  itemLabel = "rows",
  search,
  initialSorting = [],
  empty = "No results.",
  loading = false,
  footer,
}: DataTableProps<T>) {
  const [sorting, setSorting] = useState<SortingState>(initialSorting);
  const [columnFilters, setColumnFilters] = useState<ColumnFiltersState>([]);
  const [columnVisibility, setColumnVisibility] =
    useState<ColumnVisibilityState>({});
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 10,
  });
  const definitions = useMemo(() => {
    const helper = createColumnHelper<typeof features, T>();
    return helper.columns(
      columns.map((column) =>
        helper.accessor(column.value, {
          id: column.id,
          header: column.label,
          enableSorting: column.sortable !== false,
          enableHiding: column.hideable !== false,
          sortFn: "alphanumeric",
          filterFn: column.filter
            ? (row, _columnId, value: string) =>
                column.filter!.matches(row.original, value)
            : "includesString",
        }),
      ),
    );
  }, [columns]);
  const table = useTable({
    features,
    data,
    columns: definitions,
    getRowId,
    autoResetPageIndex: false,
    state: { sorting, columnFilters, columnVisibility, pagination },
    onSortingChange: (updater) => {
      setSorting(updater);
      setPagination((previous) => ({ ...previous, pageIndex: 0 }));
    },
    onColumnFiltersChange: (updater) => {
      setColumnFilters(updater);
      setPagination((previous) => ({ ...previous, pageIndex: 0 }));
    },
    onColumnVisibilityChange: setColumnVisibility,
    onPaginationChange: setPagination,
  });
  const count = table.getFilteredRowModel().rows.length;
  const pageCount = Math.max(1, table.getPageCount());
  useEffect(() => {
    // Polling must keep the current page; only clamp it when rows disappear.
    setPagination((previous) =>
      previous.pageIndex < pageCount
        ? previous
        : { ...previous, pageIndex: pageCount - 1 },
    );
  }, [pageCount]);
  const first = count ? pagination.pageIndex * pagination.pageSize + 1 : 0;
  const last = Math.min(first + pagination.pageSize - 1, count);
  const filterColumns = columns.filter((column) => column.filter);
  return (
    <div className="ui-data-table" aria-busy={loading}>
      <div className="ui-data-table-toolbar">
        {search && (
          <div className="catalog-search">
            <IconSearch size={16} aria-hidden="true" />
            <TextInput
              type="search"
              aria-label={search.label}
              placeholder={search.label}
              value={
                (table
                  .getColumn(search.columnId)
                  ?.getFilterValue() as string) ?? ""
              }
              onChange={(event) =>
                table
                  .getColumn(search.columnId)
                  ?.setFilterValue(event.target.value)
              }
            />
          </div>
        )}
        {filterColumns.map((column) => (
          <DropdownSelect
            key={column.id}
            aria-label={column.filter!.label}
            value={
              (table.getColumn(column.id)?.getFilterValue() as string) ?? ""
            }
            onChange={(event) =>
              table.getColumn(column.id)?.setFilterValue(event.target.value)
            }
          >
            {column.filter!.options.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </DropdownSelect>
        ))}
        {columnFilters.length > 0 && (
          <Button variant="tertiary" onClick={() => table.resetColumnFilters()}>
            Clear filters
          </Button>
        )}
        <div className="ui-data-table-columns">
          <DropdownMenu modal={false}>
            <DropdownMenuTrigger asChild>
              <Button>
                <IconColumns3 size={16} aria-hidden="true" />
                Columns
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" aria-label="Table columns">
              <DropdownMenuGroup>
                <DropdownMenuLabel>Visible columns</DropdownMenuLabel>
                {columns.map((definition) => {
                  const column = table.getColumn(definition.id)!;
                  return (
                    <DropdownMenuCheckboxItem
                      key={column.id}
                      checked={column.getIsVisible()}
                      disabled={!column.getCanHide()}
                      onSelect={(event) => event.preventDefault()}
                      onCheckedChange={(checked) =>
                        column.toggleVisibility(checked === true)
                      }
                    >
                      {definition.label}
                    </DropdownMenuCheckboxItem>
                  );
                })}
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
      <div className="ui-data-table-mobile-sort">
        <DropdownSelect
          aria-label="Sort rows"
          value={
            sorting[0]
              ? `${sorting[0].id}:${sorting[0].desc ? "desc" : "asc"}`
              : ""
          }
          onChange={(event) => {
            const [id, direction] = event.target.value.split(":");
            table.setSorting(id ? [{ id, desc: direction === "desc" }] : []);
          }}
        >
          <option value="">Unsorted</option>
          {columns
            .filter((column) => column.sortable !== false)
            .flatMap((column) => [
              <option key={`${column.id}:asc`} value={`${column.id}:asc`}>
                {column.label} · ascending
              </option>,
              <option key={`${column.id}:desc`} value={`${column.id}:desc`}>
                {column.label} · descending
              </option>,
            ])}
        </DropdownSelect>
      </div>
      <div className="catalog-table-container">
        <Table className="catalog-table ui-data-table-grid" aria-label={label}>
          <TableHeader>
            {table.getHeaderGroups().map((group) => (
              <TableRow key={group.id}>
                {group.headers.map((header) => {
                  const definition = columns.find(
                    (column) => column.id === header.column.id,
                  )!;
                  const order = header.column.getIsSorted();
                  return (
                    <TableHead
                      key={header.id}
                      scope="col"
                      className={definition.className}
                      aria-sort={
                        header.column.getCanSort()
                          ? order === "asc"
                            ? "ascending"
                            : order === "desc"
                              ? "descending"
                              : "none"
                          : undefined
                      }
                    >
                      {header.column.getCanSort() ? (
                        <Button
                          variant="tertiary"
                          className="ui-data-table-sort"
                          onClick={header.column.getToggleSortingHandler()}
                          aria-label={`Sort by ${definition.label}`}
                        >
                          {definition.label}
                          {order === "asc" ? (
                            <IconArrowUp size={14} aria-hidden="true" />
                          ) : order === "desc" ? (
                            <IconArrowDown size={14} aria-hidden="true" />
                          ) : (
                            <IconArrowsSort size={14} aria-hidden="true" />
                          )}
                        </Button>
                      ) : (
                        definition.label
                      )}
                    </TableHead>
                  );
                })}
              </TableRow>
            ))}
          </TableHeader>
          <TableBody>
            {loading || count === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={table.getVisibleLeafColumns().length}
                  className="ui-data-table-empty"
                >
                  {loading ? <p role="status">Loading…</p> : empty}
                </TableCell>
              </TableRow>
            ) : (
              table.getRowModel().rows.map((row) => (
                <TableRow key={row.id} className="catalog-inventory-row">
                  {row.getVisibleCells().map((cell) => {
                    const definition = columns.find(
                      (column) => column.id === cell.column.id,
                    )!;
                    return definition.rowHeader ? (
                      <TableHead
                        key={cell.id}
                        scope="row"
                        className={definition.className}
                      >
                        {definition.cell?.(row.original) ??
                          definition.value(row.original)}
                      </TableHead>
                    ) : (
                      <TableCell
                        key={cell.id}
                        data-label={
                          definition.className === "catalog-row-actions"
                            ? undefined
                            : definition.label
                        }
                        className={definition.className}
                      >
                        {definition.cell?.(row.original) ??
                          definition.value(row.original)}
                      </TableCell>
                    );
                  })}
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>
      <div className="ui-data-table-pagination">
        <span role="status">
          {first}–{last} of {count} {itemLabel}
        </span>
        <div className="ui-data-table-page-size">
          <span>Rows per page</span>
          <DropdownSelect
            aria-label="Rows per page"
            value={String(pagination.pageSize)}
            onChange={(event) =>
              table.setPagination({
                pageIndex: 0,
                pageSize: Number(event.target.value),
              })
            }
          >
            {[10, 25, 50].map((size) => (
              <option key={size} value={String(size)}>
                {size}
              </option>
            ))}
          </DropdownSelect>
        </div>
        <div className="ui-data-table-page-buttons">
          <span>
            Page {Math.min(pagination.pageIndex + 1, pageCount)} of {pageCount}
          </span>
          <Button
            aria-label="Previous page"
            disabled={loading || !table.getCanPreviousPage()}
            onClick={() => table.previousPage()}
          >
            <IconChevronLeft size={16} aria-hidden="true" />
          </Button>
          <Button
            aria-label="Next page"
            disabled={loading || !table.getCanNextPage()}
            onClick={() => table.nextPage()}
          >
            <IconChevronRight size={16} aria-hidden="true" />
          </Button>
        </div>
      </div>
      {footer}
    </div>
  );
}

import { memo } from "react";
import { debugLog } from "../../api";
import { Button } from "../../components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHeadCell,
  TableHeader,
  TableRow,
} from "../../components/ui/table";
import type { FileSearchResult } from "../../types";
import { humanSize } from "../../utils";
import type { FindSortDirection, FindSortKey } from "../FileFindingView";
import { getFileName } from "../FileFindingView";

type FindResultsTableProps = {
  visibleSearchResults: FileSearchResult[];
  searchLoading: boolean;
  findSortKey: FindSortKey;
  findSortDirection: FindSortDirection;
  onToggleSort: (key: FindSortKey) => void;
  hasMore: boolean;
  loadingMore: boolean;
  onLoadMore: () => void;
};

export const FindResultsTable = memo(
  ({
    visibleSearchResults,
    searchLoading,
    findSortKey,
    findSortDirection,
    onToggleSort,
    hasMore,
    loadingMore,
    onLoadMore,
  }: FindResultsTableProps) => {
    const renderT0 = performance.now();
    const out = (
      <div className="find-table-wrap">
        <Table className="find-table">
          <TableHeader>
            <TableRow>
              <TableHeadCell>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="find-sort"
                  onClick={() => {
                    debugLog("FileFindingView click sort name");
                    onToggleSort("name");
                  }}
                >
                  File name
                  {findSortKey === "name"
                    ? findSortDirection === "asc"
                      ? " ▲"
                      : " ▼"
                    : ""}
                </Button>
              </TableHeadCell>
              <TableHeadCell>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="find-sort"
                  onClick={() => {
                    debugLog("FileFindingView click sort size");
                    onToggleSort("size");
                  }}
                >
                  Size
                  {findSortKey === "size"
                    ? findSortDirection === "asc"
                      ? " ▲"
                      : " ▼"
                    : ""}
                </Button>
              </TableHeadCell>
              <TableHeadCell>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="find-sort"
                  onClick={() => {
                    debugLog("FileFindingView click sort path");
                    onToggleSort("path");
                  }}
                >
                  Path
                  {findSortKey === "path"
                    ? findSortDirection === "asc"
                      ? " ▲"
                      : " ▼"
                    : ""}
                </Button>
              </TableHeadCell>
            </TableRow>
          </TableHeader>
          <TableBody>
            {visibleSearchResults.map((f) => (
              <TableRow
                key={`${f.path}:${f.file_key?.dev ?? "d"}:${f.file_key?.ino ?? "i"}`}
              >
                <TableCell
                  className="find-cell-name"
                  title={getFileName(f.path)}
                >
                  {getFileName(f.path)}
                </TableCell>
                <TableCell className="find-cell-size">
                  {humanSize(f.size)}
                </TableCell>
                <TableCell
                  className="find-cell-path"
                  title={f.path}
                >
                  {f.path}
                </TableCell>
              </TableRow>
            ))}
            {visibleSearchResults.length === 0 && !searchLoading ? (
              <TableRow>
                <TableCell className="find-empty" colSpan={3}>
                  No matches yet.
                </TableCell>
              </TableRow>
            ) : null}
            {hasMore ? (
              <TableRow>
                <TableCell className="find-status-bottom" colSpan={3}>
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    disabled={loadingMore}
                    onClick={() => {
                      debugLog("FileFindingView click Load more results");
                      onLoadMore();
                    }}
                  >
                    {loadingMore ? "Loading more…" : "Load more"}
                  </Button>
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      </div>
    );
    const renderMs = performance.now() - renderT0;
    if (renderMs > 10) {
      debugLog(
        `find_ui FindResultsTable render slow ms=${renderMs.toFixed(
          1
        )} rows=${visibleSearchResults.length}`
      );
    }
    return out;
  }
);

FindResultsTable.displayName = "FindResultsTable";


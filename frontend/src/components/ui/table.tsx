import type {
  HTMLAttributes,
  TableHTMLAttributes,
  ThHTMLAttributes,
  TdHTMLAttributes,
} from "react";
import { forwardRef } from "react";

type TableProps = TableHTMLAttributes<HTMLTableElement>;
type TableHeaderProps = HTMLAttributes<HTMLTableSectionElement>;
type TableBodyProps = HTMLAttributes<HTMLTableSectionElement>;
type TableRowProps = HTMLAttributes<HTMLTableRowElement>;
type TableHeadCellProps = ThHTMLAttributes<HTMLTableCellElement>;
type TableCellProps = TdHTMLAttributes<HTMLTableCellElement>;

export const Table = forwardRef<HTMLTableElement, TableProps>(
  ({ className, ...props }, ref) => {
    const allClasses = [
      "w-full border-collapse text-sm text-slate-100",
      className ?? "",
    ]
      .filter(Boolean)
      .join(" ");
    return <table ref={ref} className={allClasses} {...props} />;
  }
);

Table.displayName = "Table";

export const TableHeader = forwardRef<
  HTMLTableSectionElement,
  TableHeaderProps
>(({ className, ...props }, ref) => {
  const allClasses = [
    "sticky top-0 bg-slate-950/90 backdrop-blur z-10",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");
  return <thead ref={ref} className={allClasses} {...props} />;
});

TableHeader.displayName = "TableHeader";

export const TableBody = forwardRef<HTMLTableSectionElement, TableBodyProps>(
  ({ className, ...props }, ref) => {
    const allClasses = [className ?? ""].filter(Boolean).join(" ");
    return <tbody ref={ref} className={allClasses} {...props} />;
  }
);

TableBody.displayName = "TableBody";

export const TableRow = forwardRef<HTMLTableRowElement, TableRowProps>(
  ({ className, ...props }, ref) => {
    const allClasses = [
      "border-b border-slate-800 hover:bg-slate-900/60",
      className ?? "",
    ]
      .filter(Boolean)
      .join(" ");
    return <tr ref={ref} className={allClasses} {...props} />;
  }
);

TableRow.displayName = "TableRow";

export const TableHeadCell = forwardRef<
  HTMLTableCellElement,
  TableHeadCellProps
>(({ className, ...props }, ref) => {
  const allClasses = [
    "px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-slate-400",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");
  return <th ref={ref} className={allClasses} {...props} />;
});

TableHeadCell.displayName = "TableHeadCell";

export const TableCell = forwardRef<HTMLTableCellElement, TableCellProps>(
  ({ className, ...props }, ref) => {
    const allClasses = [
      "px-3 py-1.5 align-middle",
      className ?? "",
    ]
      .filter(Boolean)
      .join(" ");
    return <td ref={ref} className={allClasses} {...props} />;
  }
);

TableCell.displayName = "TableCell";


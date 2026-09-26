/** 旧 CSV 用例 / 测试结果表格（设计文档 §8.4「用例」「测试」页签）。 */
export function CsvTableView({
  header,
  rows,
  emptyText,
}: {
  header: string[];
  rows: string[][];
  emptyText: string;
}) {
  if (header.length === 0) {
    return <p className="m-0 text-sm text-low">{emptyText}</p>;
  }
  return (
    <div className="overflow-x-auto rounded-sm border border-border">
      <table
        data-testid="csv-table"
        className="w-full border-collapse text-left text-sm"
      >
        <thead className="bg-secondary">
          <tr>
            {header.map((cell, index) => (
              <th
                key={index}
                scope="col"
                className="whitespace-nowrap border-b border-border px-base py-half font-medium text-high"
              >
                {cell}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, rowIndex) => (
            <tr
              key={rowIndex}
              className="border-b border-border last:border-b-0"
            >
              {header.map((_, cellIndex) => (
                <td
                  key={cellIndex}
                  className="whitespace-pre-wrap px-base py-half align-top text-normal"
                >
                  {row[cellIndex] ?? ''}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export type DataSourceMode = 'local' | 'remote';

// 默认 remote：不显式调用 configureDataSource 时，团队版行为完全不变。
let dataSourceMode: DataSourceMode = 'remote';

export function configureDataSource(mode: DataSourceMode): void {
  dataSourceMode = mode;
}

export function getDataSourceMode(): DataSourceMode {
  return dataSourceMode;
}

export function isLocalMode(): boolean {
  return dataSourceMode === 'local';
}

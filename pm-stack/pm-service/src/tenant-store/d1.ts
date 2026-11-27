/** D1 adapter for the pm-service async read path.
 *
 * The PM read store issues SELECTs only. This adapter takes the tenant's
 * Worker D1 binding directly, so the query path needs no account token or
 * database URL. The Turso HTTP adapter remains available for compatibility
 * while existing data is copied and the binding is rolled out.
 */
import type { AsyncSqlDb, AsyncSqlValue } from "./libsql-http.ts";

export interface D1PreparedStatementLike {
  bind(...values: AsyncSqlValue[]): D1PreparedStatementLike;
  all<T>(): Promise<{ success: boolean; results?: T[] }>;
}

export interface D1DatabaseLike {
  prepare(sql: string): D1PreparedStatementLike;
}

export function d1AsyncDb(database: D1DatabaseLike): AsyncSqlDb {
  return {
    async all<T>(sql: string, params: readonly AsyncSqlValue[] = []): Promise<T[]> {
      const statement = database.prepare(sql);
      const prepared = params.length > 0 ? statement.bind(...params) : statement;
      const result = await prepared.all<T>();
      if (result.success !== true || !Array.isArray(result.results)) {
        throw new Error("D1 read failed or returned a malformed result");
      }
      return result.results;
    },
  };
}

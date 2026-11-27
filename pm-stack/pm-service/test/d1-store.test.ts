// Oracle: the D1 prepared statement receives the same SQL parameters and
// rows that the async read-store interface asks for.
import { expect, test } from "bun:test";
import { d1AsyncDb, type D1DatabaseLike } from "../src/tenant-store/d1.ts";

test("D1 async adapter binds parameters and returns result rows", async () => {
  let preparedSql = "";
  let bound: unknown[] = [];
  const rows = [{ project_id: "pm" }];
  const database: D1DatabaseLike = {
    prepare(sql) {
      preparedSql = sql;
      const statement = {
        bind(...values: unknown[]) {
          bound = values;
          return statement;
        },
        async all<T>() {
          return { success: true, results: rows as T[] };
        },
      };
      return statement;
    },
  };

  const result = await d1AsyncDb(database).all<(typeof rows)[number]>("SELECT project_id WHERE project_id = ?", ["pm"]);
  expect(preparedSql).toBe("SELECT project_id WHERE project_id = ?");
  expect(bound).toEqual(["pm"]);
  expect(result).toEqual(rows);
});

test("D1 async adapter refuses failed or malformed results", async () => {
  const database: D1DatabaseLike = {
    prepare: () => ({
      bind() {
        return this;
      },
      async all() {
        return { success: false, results: [] };
      },
    }),
  };

  await expect(d1AsyncDb(database).all("SELECT 1")).rejects.toThrow("D1 read failed or returned a malformed result");
});

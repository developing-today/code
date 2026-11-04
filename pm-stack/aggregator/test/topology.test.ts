import { expect, test } from "bun:test";
import { loadConfig } from "../src/config.ts";
import { buildServices } from "../src/topology.ts";

function ports(env: Record<string, string | undefined> = { PM_AGG_DEV: "1" }) {
  return Object.fromEntries(buildServices(loadConfig(env)).map(({ id, port }) => [id, port]));
}

test("default topology ports stay at their documented values", () => {
  expect(ports()).toEqual({
    aggregator: 4870,
    kaneo: 5173,
    "kaneo-postgres": 5432,
    "ready-for-agent": 6056,
    pad: 7777,
    grist: 8484,
    "fleet-feed": null,
    orchestration: null,
    projector: 7801,
  });
});

test("probed service ports follow explicit configured URL ports", () => {
  expect(
    ports({
      PM_AGG_SOURCE: "live",
      KANEO_URL: "http://127.0.0.1:6000",
      PM_AGG_RFA_URL: "http://127.0.0.1:7778",
      PM_AGG_PAD_URL: "http://127.0.0.1:3131",
      GRIST_URL: "http://127.0.0.1:9000",
      PM_PROJECTOR_URL: "http://127.0.0.1:9999",
    }),
  ).toMatchObject({
    kaneo: 6000,
    "ready-for-agent": 7778,
    pad: 3131,
    grist: 9000,
    projector: 9999,
  });
});

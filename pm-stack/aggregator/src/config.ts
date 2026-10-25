// Environment configuration. Secrets are read by NAME from the environment
// only — never committed, never printed. Everything has a fixture-safe default.
const s = (v: string | undefined, d: string): string => (v && v.trim() !== "" ? v.trim() : d);

export interface Config {
  /** "fixtures" | "live" — the default Source. */
  source: "fixtures" | "live";
  port: number;
  kaneo: {
    baseUrl: string;
    /** Kaneo's client URL — the Origin its auth accepts (better-auth). */
    origin: string;
    email: string | null;
    password: string | null;
  };
  grist: {
    baseUrl: string;
    apiKey: string | null;
    docId: string;
  };
  projector: {
    baseUrl: string | null;
  };
}

export function loadConfig(env: Record<string, string | undefined> = process.env): Config {
  return {
    source: env.PM_AGG_SOURCE === "live" ? "live" : "fixtures",
    port: Number(s(env.PM_AGG_PORT, "4870")),
    kaneo: {
      baseUrl: s(env.KANEO_URL, "http://127.0.0.1:5173"),
      // Kaneo's auth accepts only its configured client URL as Origin.
      origin: s(env.KANEO_ORIGIN, s(env.KANEO_CLIENT_URL, "https://kaneo.security.cab")),
      email: env.KANEO_ADMIN_EMAIL ?? null,
      password: env.KANEO_ADMIN_PASSWORD ?? null,
    },
    grist: {
      baseUrl: s(env.GRIST_URL, "http://127.0.0.1:8484"),
      apiKey: env.GRIST_API_KEY ?? null,
      // The "Program roadmap" document seeded in Phase 2.
      docId: s(env.GRIST_ROADMAP_DOC, "czYHYKgG9SHcm9CPwiKEq1"),
    },
    projector: {
      // Session J's projector serves on 127.0.0.1:7801 by default; set
      // PM_PROJECTOR_URL=off to disable the live registry/item/comment reads.
      baseUrl:
        env.PM_PROJECTOR_URL === "off" || env.PM_PROJECTOR_URL === "none"
          ? null
          : s(env.PM_PROJECTOR_URL, "http://127.0.0.1:7801"),
    },
  };
}

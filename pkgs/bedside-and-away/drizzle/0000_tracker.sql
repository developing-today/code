CREATE TABLE stores (
 id TEXT PRIMARY KEY NOT NULL,
 name TEXT NOT NULL,
 type TEXT NOT NULL DEFAULT 'private' CHECK(type IN ('private','shared')),
 owner_id TEXT NOT NULL,
 archived INTEGER NOT NULL DEFAULT 0,
 state_json TEXT NOT NULL,
 version INTEGER NOT NULL DEFAULT 0,
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL
);
--> statement-breakpoint
CREATE TABLE memberships (
 store_id TEXT NOT NULL REFERENCES stores(id),
 user_id TEXT NOT NULL,
 email TEXT NOT NULL,
 role TEXT NOT NULL CHECK(role IN ('reader','editor','admin')),
 created_at TEXT NOT NULL,
 PRIMARY KEY(store_id,user_id)
);
--> statement-breakpoint
CREATE TABLE user_settings (
 user_id TEXT PRIMARY KEY NOT NULL,
 settings_json TEXT NOT NULL DEFAULT '{}',
 updated_at TEXT NOT NULL
);
--> statement-breakpoint
CREATE TABLE events (
 id TEXT PRIMARY KEY NOT NULL,
 store_id TEXT NOT NULL REFERENCES stores(id),
 version INTEGER NOT NULL,
 actor_id TEXT NOT NULL,
 actor_email TEXT NOT NULL,
 actor_name TEXT NOT NULL,
 at TEXT NOT NULL,
 payload_json TEXT NOT NULL,
 UNIQUE(store_id,version)
);
--> statement-breakpoint
CREATE TABLE invitations (
 id TEXT PRIMARY KEY NOT NULL,
 store_id TEXT NOT NULL REFERENCES stores(id),
 email TEXT NOT NULL,
 role TEXT NOT NULL CHECK(role IN ('reader','editor','admin')),
 status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','accepted','declined','revoked')),
 created_by TEXT NOT NULL,
 created_at TEXT NOT NULL,
 responded_at TEXT
);
--> statement-breakpoint
CREATE INDEX memberships_user ON memberships(user_id);
--> statement-breakpoint
CREATE INDEX events_store ON events(store_id,version);
--> statement-breakpoint
CREATE INDEX invitations_email ON invitations(email,status);
--> statement-breakpoint
CREATE TABLE access_events (
 id TEXT PRIMARY KEY NOT NULL,
 store_id TEXT NOT NULL REFERENCES stores(id),
 actor_id TEXT NOT NULL,
 at TEXT NOT NULL,
 action TEXT NOT NULL,
 details_json TEXT NOT NULL
);

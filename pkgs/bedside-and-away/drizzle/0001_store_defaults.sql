ALTER TABLE stores ADD COLUMN is_open INTEGER NOT NULL DEFAULT 0;
--> statement-breakpoint
CREATE TABLE default_stores (
 key TEXT PRIMARY KEY NOT NULL,
 store_id TEXT NOT NULL REFERENCES stores(id)
);

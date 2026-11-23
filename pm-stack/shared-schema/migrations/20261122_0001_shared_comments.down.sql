-- #1744 down migration for shared_comments. Refuses to drop the table
-- while it still holds rows: a rollback must export/replay the rows to
-- the comments.json file store first (the file backend is the rollback
-- target, and it must not silently lose comments the DB is ahead of).
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM public.shared_comments) THEN
    RAISE EXCEPTION 'refusing to drop non-empty shared_comments; export rows to the comments.json file store first';
  END IF;
END $$;

DROP TABLE public.shared_comments;

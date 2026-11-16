-- #6815 down migration. Refuses to drop any table that still holds rows:
-- a rollback must export/replay the rows to the file stores first
-- (scripts/migrate-stores-to-supabase.ts has the same mapping in reverse
-- only by reading the files, so a manual export is required if the DB
-- is ahead of the files).
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM public.agg_activity_ingestion)
     OR EXISTS (SELECT 1 FROM public.agg_activity_gaps)
     OR EXISTS (SELECT 1 FROM public.agg_usage_snapshots)
     OR EXISTS (SELECT 1 FROM public.agg_store_documents) THEN
    RAISE EXCEPTION 'refusing to drop non-empty aggregator store tables; export rows to the file stores first';
  END IF;
END $$;

DROP TABLE public.agg_store_documents;
DROP TABLE public.agg_usage_snapshots;
DROP TABLE public.agg_activity_gaps;
DROP TABLE public.agg_activity_ingestion;

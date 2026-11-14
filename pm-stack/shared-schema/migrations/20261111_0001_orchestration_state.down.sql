-- #5005 rollback guard: never discard rows during a schema rollback.
DO $$
BEGIN
  IF (to_regclass('public.orch_items') IS NOT NULL
        AND EXISTS (SELECT 1 FROM public.orch_items LIMIT 1))
     OR (to_regclass('public.orch_runs') IS NOT NULL
        AND EXISTS (SELECT 1 FROM public.orch_runs LIMIT 1)) THEN
    RAISE EXCEPTION 'refusing to drop non-empty orchestration tables; export/replay rows first';
  END IF;
END
$$;

DROP TABLE IF EXISTS public.orch_items;
DROP TABLE IF EXISTS public.orch_runs;

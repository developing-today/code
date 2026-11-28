-- #6103 rollback guard: never discard rows during a schema rollback.
DO $$
BEGIN
  IF (to_regclass('public.shared_events') IS NOT NULL
        AND EXISTS (SELECT 1 FROM public.shared_events LIMIT 1))
     OR (to_regclass('public.agg_actions') IS NOT NULL
        AND EXISTS (SELECT 1 FROM public.agg_actions LIMIT 1)) THEN
    RAISE EXCEPTION 'refusing to drop non-empty action/event tables; export/replay rows first';
  END IF;
END
$$;

DROP TABLE IF EXISTS public.shared_events;
DROP TABLE IF EXISTS public.agg_actions;

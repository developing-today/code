-- #6103 rollback guard: never discard rows during a schema rollback.
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM public.shared_events LIMIT 1)
     OR EXISTS (SELECT 1 FROM public.agg_actions LIMIT 1) THEN
    RAISE EXCEPTION 'refusing to drop non-empty action/event tables; export/replay rows first';
  END IF;
END
$$;

DROP TABLE public.shared_events;
DROP TABLE public.agg_actions;

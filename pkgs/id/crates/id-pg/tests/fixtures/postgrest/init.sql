-- Deterministic PostgREST contract surface for design #5805 §10.
CREATE ROLE authenticator NOINHERIT LOGIN PASSWORD 'fixture_password';
CREATE ROLE fixture_reader NOLOGIN;
GRANT fixture_reader TO authenticator;
CREATE SCHEMA api;
GRANT USAGE ON SCHEMA api TO fixture_reader;
CREATE SCHEMA fixture_private;
GRANT USAGE ON SCHEMA fixture_private TO fixture_reader;

CREATE TYPE api.fixture_state AS ENUM ('ready', 'done');
GRANT USAGE ON TYPE api.fixture_state TO fixture_reader;
CREATE VIEW api.contract_values AS
SELECT 1::bigint AS id,
       true::boolean AS bool_v,
       7::smallint AS int2_v,
       70000::integer AS int4_v,
       9223372036854775000::bigint AS int8_v,
       1.25::real AS float4_v,
       2.5::double precision AS float8_v,
       12345678901234567890.123456789::numeric AS numeric_v,
       'hello'::text AS text_v,
       'varchar'::varchar AS varchar_v,
       'x'::char(1) AS char_v,
       'ready'::api.fixture_state AS enum_v,
       decode('00ff1042', 'hex') AS bytea_v,
       '2024-02-03 04:05:06.123456+02'::timestamptz AS timestamptz_v,
       '2024-02-29'::date AS date_v,
       '12:34:56.123456'::time AS time_v,
       '12:34:56+02'::timetz AS timetz_v,
       interval '1 day 02:03:04' AS interval_v,
       '{"n":7}'::json AS json_v,
       '{"ok":true}'::jsonb AS jsonb_v,
       '{"label":"row"}'::json AS json_record_v,
       '[1,2,3]'::jsonb AS jsonb_list_v,
       'A0B1C2D3-E4F5-4678-9ABC-0123456789AB'::uuid AS uuid_v,
       ARRAY[1,2,3]::integer[] AS int4_array_v,
       NULL::text AS null_v;
CREATE VIEW api.nan_value AS SELECT 1::bigint AS id, 'NaN'::double precision AS v;
-- Helpers live outside the only exposed schema: the reader has no EXECUTE
-- privilege on any api object and cannot call these through /rpc.
CREATE FUNCTION fixture_private.raise_fixture_error() RETURNS double precision LANGUAGE plpgsql STABLE AS $$
BEGIN
  RAISE EXCEPTION USING ERRCODE = 'PT503', MESSAGE = 'contract fixture failure';
END;
$$;
CREATE FUNCTION fixture_private.slow_value() RETURNS double precision LANGUAGE plpgsql VOLATILE AS $$
BEGIN
  PERFORM pg_sleep(5);
  RETURN 1.0;
END;
$$;
CREATE VIEW api.sql_failure AS SELECT 1::bigint AS id, fixture_private.raise_fixture_error() AS v;
CREATE VIEW api.slow_value AS SELECT 1::bigint AS id, fixture_private.slow_value() AS v;

GRANT SELECT ON ALL TABLES IN SCHEMA api TO fixture_reader;
ALTER DEFAULT PRIVILEGES IN SCHEMA api GRANT SELECT ON TABLES TO fixture_reader;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA fixture_private FROM PUBLIC;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA fixture_private TO fixture_reader;

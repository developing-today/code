import os
import kuzu
import datetime
import importlib.util
import re
from ksuid import KsuidMs
from typing import Callable
from cuid2 import cuid_wrapper

cuid_generator: Callable[[], str] = cuid_wrapper()


def id():
    return secure_random_id()


def secure_random_id():
    return cuid_generator()


def fast_time_id():
    return KsuidMs()


def print_call_info(conn):
    db_version = execute_into_single(conn, "CALL DB_VERSION() RETURN *;")
    print("Database Version:", db_version)

    settings = execute_into_single(conn, "CALL CURRENT_SETTING('threads') RETURN *;")
    print("Threads:", settings)

    settings = execute_into_single(conn, "CALL CURRENT_SETTING('timeout') RETURN *;")
    print("Timeout:", settings)

    settings = execute_into_single(
        conn, "CALL CURRENT_SETTING('var_length_extend_max_depth') RETURN *;"
    )
    print("Var Length Extend Max Depth:", settings)

    settings = execute_into_single(
        conn, "CALL CURRENT_SETTING('enable_semi_mask') RETURN *;"
    )
    print("Enable Semi Mask:", settings)

    tables = execute_into_list(conn, "CALL SHOW_TABLES() RETURN *;")
    print("Tables in the Database:")
    for table in tables:
        print(table)


def custom_sort_key(filename, separators=None):
    if separators is None:
        separators = [r"\W+"]  # Default to non-alphanumeric characters as separators

    pattern = "|".join(separators)
    parts = re.split(pattern, os.path.splitext(filename)[0])

    return [int(part) if part.isdigit() else -1 for part in parts]


def sort_migrations(migrations_directory, separators=None):
    migrations = [
        f.name
        for f in os.scandir(migrations_directory)
        if f.is_file() and f.name.endswith(".py")
    ]
    migrations.sort(key=lambda filename: custom_sort_key(filename, separators))
    return migrations


def execute_into_single(conn, query):
    result = execute(conn, query)

    if not result.has_next():
        return None

    else:
        return result.get_next()


def execute_into_list(conn, query):
    result = execute(conn, query)

    if not result.has_next():
        return []

    else:
        result_list = []

        while result.has_next():
            result_list.append(result.get_next())

        return result_list


def execute(conn, query, **options):
    prefix = options.get("prefix", "Executing query: ")
    max_print_length = options.get("max_print_length", -1)

    if max_print_length > 0:
        combined_query = prefix + query
        display_query = (
            combined_query[:max_print_length] + "..."
            if len(combined_query) > max_print_length
            else combined_query
        )
        print(display_query)
    elif max_print_length == -1:
        print(prefix + query)

    return conn.execute(query)


def is_valid_migration_name(filename, migration_count, counter, valid_new_counters):
    match = re.match(r"(\d+)", os.path.splitext(filename)[0])

    if not match:
        return None, f"Filename does not start with a number: {filename}"

    version_number = int(match.group(1))

    if version_number <= counter:
        return (
            version_number,
            f"Version number ({version_number}) lower than or equal to current counter ({counter}): {filename}",
        )

    if version_number > counter + migration_count:
        return (
            version_number,
            f"Version number ({version_number}) higher than current counter ({counter}) + migration count ({counter + migration_count}) = ({counter + migration_count}): {filename}",
        )

    if any(version_number <= new_counter for new_counter in valid_new_counters):
        return (
            version_number,
            f"Bad version number: duplicate or lower than another new counter ({version_number}): {filename}",
        )

    return version_number, ""


def ts():
    return iso_timestamp_now()


def iso_timestamp_now():
    return datetime.datetime.now().isoformat()


def run_migrations(conn, migrations_directory, migrations, counter):
    migration_count = len(migrations) - counter
    errors = []

    print("Checking {} migrations...".format(migration_count))

    valid_new_counters = []
    for migration in migrations[counter:]:
        new_counter, error_message = is_valid_migration_name(
            migration, migration_count, counter, valid_new_counters
        )
        if new_counter is None or error_message != "":
            errors.append((migration, error_message, new_counter))
        else:
            valid_new_counters.append(new_counter)

    if len(valid_new_counters) + len(errors) < migration_count:
        errors.append((None, "Missing version number", None))

    if errors:
        error_messages = "\n".join(
            [
                f"Migration {name}({counter}): {message}"
                for name, message, counter in errors
            ]
        )
        raise Exception(f"Errors encountered in migrations:\n{error_messages}")

    print("Running {} migrations...".format(migration_count))

    for migration in migrations[counter:]:
        new_version_name = os.path.splitext(migration)[0]
        timestamp_now = iso_timestamp_now()

        execute(
            conn,
            f"""
            CREATE (v:Version {{
                name: "{new_version_name}",
                executions: 1,
                completions: 0,
                created: TIMESTAMP("{timestamp_now}"),
                updated: TIMESTAMP("{timestamp_now}")
            }})
        """,
        )
        run_python_migration(migrations_directory, migration, conn)

        counter += 1

        execute(
            conn,
            f"""
            MATCH (v:Version)
            WHERE v.counter = {counter}
            SET v.completions = v.completions + 1,
                v.updated = TIMESTAMP("{timestamp_now}")
        """,
        )

    print("Migrations completed.")


def get_latest_version(conn):
    result = execute_into_single(
        conn,
        "MATCH (v:Version) RETURN v.name, v.counter, v.completions ORDER BY v.counter DESC LIMIT 1;",
    )
    if result is None:
        return None, "No Version"
    return result, f"Incomplete Version: {result}" if result[2] == 0 else ""


def create_version_table(conn):
    timestamp_now = datetime.datetime.now().isoformat()
    print("Creating version table at timestamp", timestamp_now)
    execute(
        conn,
        """
        CREATE NODE TABLE Version(
            name STRING,
            counter SERIAL,
            executions INT64,
            completions INT64,
            created TIMESTAMP,
            updated TIMESTAMP,
            PRIMARY KEY (counter))
    """,
    )


def create_version_zero(conn):
    timestamp_now = datetime.datetime.now().isoformat()
    print("Creating 0 version at timestamp", timestamp_now)
    execute(
        conn,
        f"""
        CREATE (v:Version {{
            name: "",
            executions: 0,
            completions: 0,
            created: TIMESTAMP("{timestamp_now}"),
            updated: TIMESTAMP("{timestamp_now}")
        }})
    """,
    )


def run_python_migration(migrations_directory, migration, conn):
    print("Running migration", migration)
    migration_file = os.path.join(migrations_directory, migration)
    spec = importlib.util.spec_from_file_location("migration_module", migration_file)
    migration_module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(migration_module)
    migration_module.run_migration(conn)

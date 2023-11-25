import os
import kuzu
import datetime
import importlib.util

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

def execute(conn, query):
    print("Executing query:", query)
    return conn.execute(query)

def run_migrations(conn, migrations_directory, migrations, counter):
    print("Running {} migrations...".format(len(migrations) - counter))

    for i, migration in enumerate(migrations[counter:], start=counter + 1):
        run_python_migration(migrations_directory, migration, conn)

        # Update version record
        new_version_name = os.path.splitext(migration)[0]
        timestamp_now = datetime.datetime.now().isoformat()
        execute(conn, f'''
            CREATE (v:Version {{
                name: "{new_version_name}",
                executions: 1,
                counter: {i},
                completions: 1,
                created: TIMESTAMP("{timestamp_now}"),
                updated: TIMESTAMP("{timestamp_now}")
            }})
        ''')

    print("Migrations completed.")

def get_latest_version(conn):
  return execute_into_single(conn, 'MATCH (v:Version) RETURN v.name, v.counter ORDER BY v.counter DESC LIMIT 1;')

def create_version_table(conn):
    timestamp_now = datetime.datetime.now().isoformat()
    print("Creating version table at timestamp", timestamp_now)
    execute(conn, '''
        CREATE NODE TABLE Version(
            name STRING,
            counter INT64,
            executions INT64,
            completions INT64,
            created TIMESTAMP,
            updated TIMESTAMP,
            PRIMARY KEY (counter))
    ''')

def create_version_zero(conn):
    timestamp_now = datetime.datetime.now().isoformat()
    print("Creating 0 version at timestamp", timestamp_now)
    execute(conn, f'''
        CREATE (v:Version {{
            name: "",
            executions: 0,
            counter: 0,
            completions: 0,
            created: TIMESTAMP("{timestamp_now}"),
            updated: TIMESTAMP("{timestamp_now}")
        }})
    ''')

def run_python_migration(migrations_directory, migration, conn):
    print("Running migration", migration)
    migration_file = os.path.join(migrations_directory, migration)
    spec = importlib.util.spec_from_file_location("migration_module", migration_file)
    migration_module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(migration_module)
    migration_module.run_migration(conn)
